mod cli;
mod errors;

use std::{
    env,
    fs::{create_dir_all, remove_file},
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    process::Command,
    sync::mpsc,
    thread,
};

use clap::Parser;
use directories::ProjectDirs;
use errors::Error;
use log::{debug, info, warn};

static COMMAND: &str = "wezterm connect dropdown";

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
            "open" | "close" => tx.send(command.clone()).unwrap(),
            "term" => break,
            _ => warn!("Unrecognized command: {}", command),
        }
    }
    Ok(())
}

fn run(listener: UnixListener) {
    let (tx, rx) = mpsc::channel::<String>();
    let program_manager = thread::spawn(move || {
        let mut program = Command::new("sh")
            .arg("-c")
            .arg(COMMAND)
            .spawn()
            .expect("failed to start");
        loop {
            if let Ok(msg) = rx.recv() {
                match msg.as_str() {
                    "open" => info!("Opening"),
                    "close" => {
                        info!("Closing");
                        program.kill().unwrap();
                        break;
                    }
                    msg => info!("Unknown: '{msg}'"),
                }
            }
        }
    });
    let _socket_manager = thread::spawn(|| {
        handle_socket_messages(listener, tx).unwrap();
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
