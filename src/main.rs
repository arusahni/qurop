mod cli;
mod config;
mod errors;
mod utils;
mod x11;

use std::{
    env,
    fs::{create_dir_all, remove_file},
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    panic, process,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, RwLock,
    },
    thread,
    time::{Duration, SystemTime},
};

use clap::Parser;
use directories::ProjectDirs;
use tracing::{debug, error, info, trace, warn};

use errors::Error;
use tracing_subscriber::{
    fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter,
};
use utils::abort;

#[derive(Debug)]
enum StreamState {
    New(UnixListener),
    Exists(UnixStream),
}

struct Sentinel {
    flag: Arc<AtomicBool>,
}

impl Drop for Sentinel {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        warn!("A thread has terminated or panicked");
    }
}

/// Get (and possibly create) a socket for the given instance.
fn get_socket(instance_name: &str) -> Result<StreamState, Error> {
    let dir = match ProjectDirs::from("net", "arusahni", "qurop")
        .expect("could not find project dirs")
        .runtime_dir()
    {
        Some(runtime_dir) => runtime_dir.to_path_buf(),
        None => env::temp_dir().join("qurop"),
    };
    debug!(
        "[{}] Attempting to create project dir: {:?}",
        instance_name, dir
    );
    create_dir_all(&dir)?;
    let socket_path = dir.join(format!("{instance_name}.session.sock"));
    let socket_exists = socket_path.exists();
    debug!(
        "[{}] Socket: {:?} | Exists? {}",
        instance_name, socket_path, socket_exists
    );
    if socket_exists {
        debug!("[{}] Connecting to socket", instance_name);
        let socket = match UnixStream::connect(&socket_path) {
            Ok(socket) => Ok(socket),
            Err(err) => match err.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    warn!(
                        "[{}] Can't connect to socket. Assuming stale, starting new session.",
                        instance_name
                    );
                    remove_file(&socket_path)?;
                    let listener = UnixListener::bind(&socket_path)?;
                    return Ok(StreamState::New(listener));
                }
                _ => Err(err),
            },
        }?;
        Ok(StreamState::Exists(socket))
    } else {
        info!("[{}] Creating socket", instance_name);
        let listener = UnixListener::bind(&socket_path)?;
        Ok(StreamState::New(listener))
    }
}

fn handle_socket_messages(listener: UnixListener, tx: mpsc::Sender<String>) -> Result<(), Error> {
    loop {
        let (mut stream, addr) = listener.accept()?;
        debug!("Accepting stream from: {:?}", addr);
        let mut command = String::new();
        stream.read_to_string(&mut command)?;
        debug!("Received: {}", command);
        match command.as_str() {
            "open" | "toggle" | "hide" | "kill" => {
                tx.send(command.clone()).expect("command should send")
            }
            "term" => break,
            _ => warn!("Unrecognized command: {}", command),
        }
    }
    Ok(())
}

/// Find and position the window
fn block_for_window(matcher: &x11::WindowMatcher, window_delay: Option<u64>) -> u32 {
    trace!("blocking for window {:?}", matcher);
    let mut count = 0;
    let start = SystemTime::now();
    loop {
        match x11::map_qurop_window(matcher) {
            Ok(window_id) => {
                x11::position_window(window_id, window_delay);
                return window_id;
            }
            Err(Error::WindowNotFound) => {
                trace!("window not found");
            }
            Err(err) => panic!("Unhandled error: {err}"),
        }
        count += 1;
        if count % 5 == 4 {
            warn!("could not find window in {} attempts", count);
            thread::sleep(Duration::from_millis(100));
        }
        if SystemTime::now().duration_since(start).unwrap() > Duration::from_secs(5) {
            panic!(
                "could not find window after 5 seconds and {} attempts",
                count
            );
        }
    }
}

pub(crate) struct Context {
    pub matcher: x11::WindowMatcher,
    pub window_id: Option<u32>,
}

pub(crate) fn program_thread(
    rx: mpsc::Receiver<String>,
    instance: Instance,
    ctx: &Arc<RwLock<Context>>,
) {
    let mut program = process::Command::new("sh")
        .arg("-c")
        .arg(instance.command.clone())
        .spawn()
        .expect("failed to start");
    info!("[{}] Started PID: {}", instance.name, program.id());
    {
        let write_ctx = &mut ctx.write().unwrap();
        if matches!(write_ctx.matcher, x11::WindowMatcher::ProcessId(_)) {
            write_ctx.matcher = x11::WindowMatcher::ProcessId(Some(program.id()));
            trace!("[{}] Set a new PID {}", instance.name, program.id());
        }
        write_ctx.window_id = Some(block_for_window(&write_ctx.matcher, instance.window_delay));
        trace!(
            "[{}] Set a new Window ID {:?}",
            instance.name,
            write_ctx.window_id
        );
    }
    loop {
        if let Ok(msg) = rx.recv() {
            let action = if msg == "toggle" {
                let win_id = ctx.read().unwrap().window_id;
                if win_id.is_some() && x11::window_is_active(win_id.unwrap()) {
                    "hide".into()
                } else {
                    "open".into()
                }
            } else {
                msg.clone()
            };
            debug!("[{}] Taking action: '{}'", instance.name, action);
            match action.as_str() {
                "open" => {
                    let read_ctx = ctx.clone();
                    if let Ok(Some(status)) = program.try_wait() {
                        info!(
                            "[{}] Program has exited ({}). Restarting.",
                            instance.name, status
                        );
                        let mut write_ctx = read_ctx.write().unwrap();
                        program = process::Command::new("sh")
                            .arg("-c")
                            .arg(instance.command.clone())
                            .spawn()
                            .expect("failed to start");
                        if matches!(write_ctx.matcher, x11::WindowMatcher::ProcessId(_)) {
                            trace!("[{}] Setting new pid {}", instance.name, program.id());
                            write_ctx.matcher = x11::WindowMatcher::ProcessId(Some(program.id()));
                        }
                        write_ctx.window_id =
                            Some(block_for_window(&write_ctx.matcher, instance.window_delay));
                    } else {
                        let window_id = read_ctx.read().unwrap().window_id.unwrap();
                        x11::map_window(window_id);
                        x11::position_window(window_id, instance.window_delay);
                    }
                }
                "kill" => {
                    info!("[{}] Killing", instance.name);
                    program.kill().unwrap();
                    break;
                }
                "hide" => {
                    let local_ctx = ctx.clone();
                    let read_ctx = local_ctx.read().unwrap();
                    match read_ctx.window_id {
                        Some(window_id) => x11::unmap_window(window_id),
                        None => x11::unmap_qurop_window(&read_ctx.matcher),
                    }
                }
                command if command.starts_with("hide:") => {
                    x11::unmap_window(command.split(':').last().unwrap().parse().unwrap())
                }
                _ => info!("[{}] Unknown: '{}' ({})", instance.name, msg, action),
            }
        }
    }
}

fn run(listener: UnixListener, instance: Instance) {
    let flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<String>();
    let ctx = Arc::new(RwLock::new(Context {
        matcher: instance.matcher.clone(),
        window_id: None,
    }));
    let program_ctx = Arc::clone(&ctx);
    let program_flag = Arc::clone(&flag);
    let _program_manager = thread::spawn(move || {
        let _sentinel = Sentinel { flag: program_flag };
        match panic::catch_unwind(|| {
            program_thread(rx, instance, &program_ctx);
        }) {
            Ok(()) => info!("Program thread completed"),
            Err(err) => error!(?err, "Program thread panicked"),
        };
    });
    let socket_tx = tx.clone();
    let socket_flag = Arc::clone(&flag);
    let _socket_manager = thread::spawn(|| {
        let _sentinel = Sentinel { flag: socket_flag };
        match panic::catch_unwind(|| {
            handle_socket_messages(listener, socket_tx).unwrap();
        }) {
            Ok(()) => info!("Socket thread completed"),
            Err(err) => error!(?err, "Socket thread panicked"),
        };
    });
    let window_ctx = Arc::clone(&ctx);
    let wm_flag = Arc::clone(&flag);
    let _window_server_manager = thread::spawn(move || {
        let _sentinel = Sentinel { flag: wm_flag };
        match panic::catch_unwind(|| {
            x11::handle_window(tx, &window_ctx);
        }) {
            Ok(()) => info!("WM thread completed"),
            Err(err) => error!(?err, "WM thread panicked"),
        };
    });
    while !flag.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(500));
    }
}

#[derive(Debug, Clone)]
struct Instance {
    name: String,
    command: String,
    matcher: x11::WindowMatcher,
    window_delay: Option<u64>,
}

fn main() -> Result<(), Error> {
    let args = cli::Args::parse();
    if let Some(level) = args.persist_verbosity {
        println!("Level: {:?}", level);
        let project =
            ProjectDirs::from("net", "arusahni", "qurop").expect("could not find project dirs");
        let state_dir = project.state_dir().unwrap();
        let logfile =
            tracing_appender::rolling::hourly(state_dir, "main.log").with_max_level(level);
        tracing_subscriber::fmt()
            .with_writer(std::io::stdout.and(logfile))
            .with_env_filter(EnvFilter::from_env("QUROP_LOG"))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(EnvFilter::from_env("QUROP_LOG"))
            .init();
    }
    let config = config::get_config().unwrap_or_else(|err| panic!("Invalid configuration: {err}"));
    let (action, instance_name) = match args.command {
        cli::Command::Add {
            name,
            command,
            matcher,
            class_name,
            ..
        } => {
            config::add_instance(&name, &command.join(" "), matcher, class_name)?;
            process::exit(0);
        }
        cli::Command::Open { name } => ("open", name),
        cli::Command::Kill { name } => ("kill", name),
        cli::Command::Hide { name } => ("hide", name),
        cli::Command::Toggle { name } => ("toggle", name),
    };
    let instance = config.instances.get(&instance_name).unwrap_or_else(|| {
        abort(&format!(
            "No configuration found for '{}'. Add it via `qurop add {} <command>`",
            instance_name, instance_name
        ))
    });
    let instance = Instance {
        name: instance_name.clone(),
        command: instance.command.clone(),
        matcher: match instance.matcher {
            config::WindowMatcher::Class => x11::WindowMatcher::WmClass(
                instance
                    .class_name
                    .clone()
                    .unwrap_or_else(|| abort("'class_name' must be specified")),
            ),
            config::WindowMatcher::Process => x11::WindowMatcher::ProcessId(None),
        },
        window_delay: instance.window_delay_ms.or(Some(100)),
    };
    match get_socket(&instance_name)? {
        StreamState::Exists(mut stream) => {
            info!("[{}] Launching client", instance_name);
            stream.write_all(action.to_string().as_bytes())?;
            return Ok(());
        }
        StreamState::New(listener) => {
            info!("[{}] Launching server", instance_name);
            run(listener, instance.clone());
        }
    };
    Ok(())
}
