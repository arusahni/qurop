use std::{
    sync::{mpsc, Arc, RwLock},
    thread,
    time::{Duration, SystemTime},
};
use tracing::{debug, info, trace, warn};
use x11rb::{
    connection::Connection,
    properties::WmClass,
    protocol::{xproto::*, Event},
    wrapper::ConnectionExt as WrapperConnectionExt,
};

use crate::errors::Error;

x11rb::atom_manager! {
    pub(crate) Atoms:
    AtomsCookie {
        _MOTIF_WM_HINTS,
        _NET_ACTIVE_WINDOW,
        _NET_WM_CLASS,
        _NET_WM_NAME,
        _NET_WM_PID,
        _NET_WM_WINDOW_TYPE,
        _KDE_NET_WM_WINDOW_TYPE_OVERRIDE,
        UTF8_STRING,
    }
}

trait QuropConnectionExt {
    fn flush_and_sync(&self);
}

impl QuropConnectionExt for x11rb::rust_connection::RustConnection {
    fn flush_and_sync(&self) {
        self.flush().expect("couldn't flush to x11");
        self.sync().expect("couldn't sync with x11");
    }
}

/// Find the window that is currently active on the given screen
pub(crate) fn get_active_window(
    connection: &x11rb::rust_connection::RustConnection,
    screen: &Screen,
    atom: u32,
) -> Result<u32, ()> {
    let response = connection
        .get_property(false, screen.root, atom, AtomEnum::WINDOW, 0, 1)
        .unwrap()
        .reply()
        .unwrap();

    if response.value32().is_none() {
        return Err(());
    }

    Ok(response.to_owned().value32().unwrap().next().unwrap())
}

#[derive(Debug, Clone)]
pub(crate) enum WindowMatcher {
    ProcessId(Option<u32>),
    WmClass(String),
}

fn query_windows(connection: &x11rb::rust_connection::RustConnection, root: u32) -> Vec<u32> {
    let tree = connection
        .query_tree(root)
        .expect("unable to query tree")
        .reply()
        .unwrap();
    let mut windows = vec![];
    for child in tree.children {
        windows.push(child);
        windows.extend(query_windows(connection, child));
    }
    windows
}

/// Get the ID of the managed window
pub(crate) fn get_qurop_window_id(
    connection: &x11rb::rust_connection::RustConnection,
    root: u32,
    matcher: &WindowMatcher,
) -> Option<u32> {
    connection.flush_and_sync();
    let atoms = Atoms::new(connection).unwrap().reply().unwrap();
    let windows = query_windows(connection, root);
    windows.into_iter().find(|child| match matcher {
        WindowMatcher::WmClass(qurop_class) => {
            if let Some(class_name) = get_window_class(connection, *child) {
                // trace!("Found class {class_name} ({child})");
                class_name == *qurop_class
            } else {
                false
            }
        }
        WindowMatcher::ProcessId(process_id) => match get_window_pid(connection, *child, atoms) {
            Some(window_process_id) => {
                // trace!("Found pid {window_process_id} ({child})");
                *process_id == Some(window_process_id)
            }
            None => false,
        },
    })
}

/// Get the name of the specified window.
pub(crate) fn get_window_name(
    conn: &x11rb::rust_connection::RustConnection,
    window_id: u32,
    atoms: Atoms,
) -> String {
    let name = conn
        .get_property(
            false,
            window_id,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            0,
            0x1000,
        )
        .unwrap();
    String::from_utf8(name.reply().unwrap().value).unwrap()
}

/// Get the class of the specified window.
pub(crate) fn get_window_class(
    conn: &x11rb::rust_connection::RustConnection,
    window_id: u32,
) -> Option<String> {
    // trace!("Getting window class for {window_id}");
    let class = WmClass::get(conn, window_id).expect("wmclass error");
    match class.reply() {
        Ok(wm_class) => Some(String::from(std::str::from_utf8(wm_class.class()).unwrap())),
        Err(_) => None,
    }
}

/// Get the class of the specified window.
pub(crate) fn get_window_pid(
    conn: &x11rb::rust_connection::RustConnection,
    window_id: u32,
    atoms: Atoms,
) -> Option<u32> {
    // trace!("Getting window pid for {window_id}");
    let property = conn.get_property(
        false,
        window_id,
        atoms._NET_WM_PID,
        AtomEnum::CARDINAL,
        0,
        1024,
    );
    match property.expect("couldn't get property").reply() {
        Ok(reply) => {
            if let Some(mut vals) = reply.value32() {
                let val = &vals.next().expect("no values");
                Some(*val)
            } else {
                // trace!("couldn't find pid for {window_id}");
                None
            }
        }
        Err(_) => None,
    }
}

/// Handle window focus changes.
pub(crate) fn handle_window(tx: mpsc::Sender<String>, ctx: &Arc<RwLock<crate::Context>>) {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let root = screen.root;
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    let active_atom = atoms._NET_ACTIVE_WINDOW;
    connection.flush_and_sync();
    let mut count = 0;
    let start = SystemTime::now();
    let window_id = loop {
        let ctx_win = { ctx.read().unwrap().window_id };
        match ctx_win {
            Some(win_id) => break win_id,
            None => {
                count += 1;
                if count % 5 == 0 {
                    warn!("window not provided in {} attempts", count);
                    thread::sleep(Duration::from_millis(100));
                    connection.flush_and_sync();
                } else if SystemTime::now().duration_since(start).unwrap() > Duration::from_secs(5)
                {
                    panic!("could not find window in 5 seconds and {} attempts", count);
                }
            }
        };
    };
    let active_class = get_window_class(&connection, window_id).unwrap_or_else(|| "unknown".into());
    let active_name = get_window_name(&connection, window_id, atoms);
    debug!("Active window: {window_id} ({active_name} | {active_class})");
    let event_sub = ChangeWindowAttributesAux::default().event_mask(EventMask::PROPERTY_CHANGE);
    connection
        .change_window_attributes(root, &event_sub)
        .expect("couldn't watch attributes");
    info!("starting waiting for events with window {}", window_id);
    connection.flush().unwrap();
    loop {
        let event = connection
            .wait_for_event()
            .expect("could not wait for xserver events");

        if let Event::PropertyNotify(e) = event {
            trace!("Property notify event for {}", e.atom);
            if e.atom == active_atom {
                if let Ok(active_window) = get_active_window(&connection, screen, active_atom) {
                    if active_window != window_id {
                        debug!("sending hide request: {} != {}", active_window, window_id);
                        tx.send(format!("hide:{window_id}"))
                            .expect("couldn't send hide command");
                    }
                }
            }
        }
    }
}

/// Determine if the window corresponding to the matcher is currently active.
pub(crate) fn window_is_active(window_id: u32) -> bool {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    connection.flush_and_sync();
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    if let Ok(active_window) = get_active_window(&connection, screen, atoms._NET_ACTIVE_WINDOW) {
        trace!("Active window ID: {}", active_window);
        return active_window == window_id;
    }
    false
}

/// Unmap the qurop window.
pub(crate) fn unmap_qurop_window(matcher: &WindowMatcher) {
    trace!("unmapping qurop");
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id = match get_qurop_window_id(&connection, screen.root, matcher) {
        Some(win_id) => win_id,
        None => {
            debug!("No window found");
            return;
        }
    };
    info!("Unmapping qurop window: {qurop_window_id}");
    connection
        .unmap_window(qurop_window_id)
        .expect("could not unmap window");
}

/// Unmap the specified window.
pub(crate) fn unmap_window(window_id: u32) {
    let (connection, _num) = x11rb::connect(None).expect("x11 connection missing");
    info!("Unmapping window: {window_id}");
    connection
        .unmap_window(window_id)
        .expect("could not unmap window");
    connection.flush_and_sync();
}

/// Map the qurop window.
pub(crate) fn map_qurop_window(matcher: &WindowMatcher) -> Result<u32, Error> {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id = get_qurop_window_id(&connection, screen.root, matcher)
        .ok_or_else(|| Error::WindowNotFound)?;
    info!("Mapping qurop window: {qurop_window_id}");
    map_window(qurop_window_id);
    Ok(qurop_window_id)
}

pub(crate) fn map_window(window_id: u32) {
    let (connection, _num) = x11rb::connect(None).expect("x11 connection missing");
    info!("Mapping window: {window_id}");
    connection
        .map_window(window_id)
        .expect("could not map window");
    connection.flush_and_sync();
}

/// Position the window and set decoration properties.
pub(crate) fn position_window(window_id: u32, window_delay: Option<u64>) {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let width = ((screen.width_in_pixels as f64) * 0.66) as u32;
    let height = ((screen.height_in_pixels as f64) * 0.5) as u32;
    let x_pos = ((screen.width_in_pixels as f64) * ((1.0 - 0.66) / 2.0)) as i32;
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    connection
        .change_property32(
            PropMode::REPLACE,
            window_id,
            atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[atoms._KDE_NET_WM_WINDOW_TYPE_OVERRIDE],
        )
        .expect("setting kwin property");
    connection
        .change_property32(
            PropMode::REPLACE,
            window_id,
            atoms._MOTIF_WM_HINTS,
            AtomEnum::CARDINAL,
            &[2, 0, 0, 0, 0],
        )
        .expect("setting motif property");
    // We split positioning and resizing due to needing a messy hack to address possible races with
    // application startup. Certain terminal emulators, such as Wezterm, don't register resize
    // events until a certain point in their startup. Sleeping for 100ms seems to handle this well
    // enough, but it adds some visual jank if the window launches centered and then snaps to the
    // top of the screen after 100ms. By immediately positioning it at the top and then resizing,
    // we can minimize the jank.
    let window_position_config = ConfigureWindowAux::new().x(Some(x_pos)).y(Some(0));
    debug!(
        "Positioning window {} to: {:?}",
        window_id, window_position_config
    );
    connection
        .configure_window(window_id, &window_position_config)
        .expect("couldn't configure window");
    connection.flush_and_sync();
    let window_geometry_config = ConfigureWindowAux::new()
        .height(Some(height))
        .width(Some(width))
        .border_width(Some(0));
    debug!(
        "Resizing window {} to: {:?}",
        window_id, window_geometry_config
    );
    connection
        .configure_window(window_id, &window_geometry_config)
        .expect("couldn't configure window");
    if let Some(window_delay_ms) = window_delay {
        // Ugly hack that makes me sad.
        thread::sleep(Duration::from_millis(window_delay_ms));
    }
    connection.flush_and_sync();
}
