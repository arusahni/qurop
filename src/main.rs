mod cli;
mod errors;
mod x11;

use std::{
    env,
    fs::{create_dir_all, remove_file},
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};

use clap::Parser;
use directories::ProjectDirs;
use log::{debug, info, trace, warn};

use errors::Error;

static COMMAND: &str = "wezterm connect dropdown --class qurop";

enum StreamState {
    New(UnixListener),
    Exists(UnixStream),
}

fn get_socket() -> Result<StreamState, Error> {
    let dir = match ProjectDirs::from("net", "arusahni", "qurop")
        .expect("could not find project dirs")
        .runtime_dir()
    {
        Some(runtime_dir) => runtime_dir.to_path_buf(),
        None => env::temp_dir().join("qurop"),
    };
    debug!("Attempting to create project dir: {:?}", dir);
    create_dir_all(&dir)?;
    let socket_path = dir.join("session.sock");
    let socket_exists = socket_path.exists();
    debug!(
        "Connecting to socket: {:?}. Exists? {}",
        socket_path, socket_exists
    );
    if socket_exists {
        debug!("Connecting to socket");
        let socket = match UnixStream::connect(&socket_path) {
            Ok(socket) => Ok(socket),
            Err(err) => match err.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    warn!("Can't connect to socket. Assuming stale, starting new session.");
                    remove_file(&socket_path)?;
                    let listener = UnixListener::bind(&socket_path)?;
                    return Ok(StreamState::New(listener));
                }
                _ => Err(err),
            },
        }?;
        Ok(StreamState::Exists(socket))
    } else {
        info!("Creating socket");
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
            "open" | "close" => tx.send(command.clone()).expect("command should send"),
            "term" => break,
            _ => warn!("Unrecognized command: {}", command),
        }
    }
    Ok(())
}

fn block_for_window() {
    trace!("blocking for window");
    let mut count = 0;
    loop {
        match x11::map_qurop_window() {
            Ok(window_id) => {
                x11::position_window(window_id);
                return;
            }
            Err(Error::WindowNotFound) => {
                trace!("window not found");
            }
            Err(err) => panic!("Unhandled error: {err}"),
        }
        count += 1;
        if count == 5 {
            warn!("could not find window in {} attempts", count);
            thread::sleep(Duration::from_millis(100));
        }
        if count > 10 {
            panic!("could not find window in {} attempts", count);
        }
    }
}

fn run(listener: UnixListener) {
    let (tx, rx) = mpsc::channel::<String>();
    let program_manager = thread::spawn(move || {
        let mut program = Command::new("sh")
            .arg("-c")
            .arg(COMMAND)
            .spawn()
            .expect("failed to start");
        block_for_window();
        loop {
            if let Ok(msg) = rx.recv() {
                match msg.as_str() {
                    "open" => {
                        if let Ok(Some(status)) = program.try_wait() {
                            info!("Program has exited ({}). Restarting.", status);
                            program = Command::new("sh")
                                .arg("-c")
                                .arg(COMMAND)
                                .spawn()
                                .expect("failed to start");
                        }
                        block_for_window();
                    }
                    "close" => {
                        info!("Closing");
                        program.kill().unwrap();
                        break;
                    }
                    "unmap" => x11::unmap_qurop_window(),
                    command if command.starts_with("unmap:") => {
                        x11::unmap_window(command.split(':').last().unwrap().parse().unwrap())
                    }
                    _ => info!("Unknown: '{}'", msg),
                }
            }
        }
    });
    let socket_tx = tx.clone();
    let _socket_manager = thread::spawn(|| {
        handle_socket_messages(listener, socket_tx).unwrap();
    });
    let _window_server_manager = thread::spawn(|| {
        x11::handle_window(tx);
    });
    program_manager.join().unwrap();
}

fn main() -> Result<(), Error> {
    env_logger::init();
    match get_socket()? {
        StreamState::Exists(mut stream) => {
            info!("Launching client");
            let args = cli::Args::parse();
            let action = args.action.unwrap_or("open".to_string());
            stream.write_all(action.as_bytes())?;
            return Ok(());
        }
        StreamState::New(listener) => {
            info!("Launching server");
            run(listener);
        }
    };
    Ok(())
}
