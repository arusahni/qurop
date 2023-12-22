use log::{debug, info, log_enabled, trace, warn};
use std::{
    sync::{mpsc, Arc, RwLock},
    thread,
    time::Duration,
};
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

/// Get the ID of the managed window
pub(crate) fn get_qurop_window_id(
    connection: &x11rb::rust_connection::RustConnection,
    root: u32,
    matcher: &WindowMatcher,
) -> Option<u32> {
    connection.flush().expect("couldn't flush");
    connection.sync().expect("couldn't sync");
    let response = connection
        .query_tree(root)
        .expect("unable to query tree")
        .reply()
        .unwrap();
    let atoms = Atoms::new(connection).unwrap().reply().unwrap();
    response.children.into_iter().find(|child| match matcher {
        WindowMatcher::WmClass(qurop_class) => {
            if let Some(class_name) = get_window_class(connection, *child) {
                trace!("Found class {class_name} ({child})");
                class_name == *qurop_class
            } else {
                false
            }
        }
        WindowMatcher::ProcessId(process_id) => match get_window_pid(connection, *child, atoms) {
            Some(window_process_id) => *process_id == Some(window_process_id),
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
    if let Some(mut vals) = property
        .expect("couldn't get property")
        .reply()
        .expect("couldn't read response")
        .value32()
    {
        let val = &vals.next().expect("no values");
        Some(*val)
    } else {
        // trace!("couldn't find value");
        None
    }
}

/// Handle window focus changes.
pub(crate) fn handle_window(tx: mpsc::Sender<String>, ctx: &Arc<RwLock<crate::Context>>) {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let root = screen.root;
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    let active_atom = atoms._NET_ACTIVE_WINDOW;
    debug!(
        "window handler: Root {:?} | active_atom {}",
        root, active_atom
    );
    let mut count = 0;
    let window_id = loop {
        trace!("window handler blocking for window id");
        {
            let matcher = &ctx.read().unwrap().matcher;
            let window_id = get_qurop_window_id(&connection, root, matcher);
            if let Some(window_id) = window_id {
                break window_id;
            }
        }
        count += 1;
        if count == 5 {
            warn!("could not find window in {} attempts", count);
            thread::sleep(Duration::from_millis(100));
        }
        if count > 10 {
            panic!("could not find window in {} attempts", count);
        }
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
                        debug!("sending unmap request: {} != {}", active_window, window_id);
                        tx.send(format!("unmap:{window_id}"))
                            .expect("couldn't send unmap command");
                    }
                }
            }
        }
    }
}

/// Unmap the qurop window.
pub(crate) fn unmap_qurop_window(matcher: &WindowMatcher) {
    trace!("unmapping qurop");
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id =
        get_qurop_window_id(&connection, screen.root, matcher).expect("could not find window");
    info!("Unmapping qurop window: {qurop_window_id}");
    connection
        .unmap_window(qurop_window_id)
        .expect("could not unmap window");
    connection.flush().expect("couldn't flush");
}

/// Unmap the specified window.
pub(crate) fn unmap_window(window_id: u32) {
    let (connection, _num) = x11rb::connect(None).expect("x11 connection missing");
    info!("Unmapping window: {window_id}");
    connection
        .unmap_window(window_id)
        .expect("could not unmap window");
    connection.flush().expect("couldn't flush");
}

/// Map the qurop window.
pub(crate) fn map_qurop_window(matcher: &WindowMatcher) -> Result<u32, Error> {
    trace!("mapping qurop");
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id = get_qurop_window_id(&connection, screen.root, matcher)
        .ok_or_else(|| Error::WindowNotFound)?;
    info!("Mapping qurop window: {qurop_window_id}");
    connection
        .map_window(qurop_window_id)
        .expect("could not map window");
    connection.flush().expect("couldn't flush");
    Ok(qurop_window_id)
}

/// Position the window and set decoration properties.
pub(crate) fn position_window(window_id: u32) {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let width = ((screen.width_in_pixels as f64) * 0.66) as u32;
    let height = ((screen.height_in_pixels as f64) * 0.5) as u32;
    let x_pos = ((screen.width_in_pixels as f64) * ((1.0 - 0.66) / 2.0)) as i32;
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    let window_config = ConfigureWindowAux::new()
        .height(Some(height))
        .width(Some(width))
        .border_width(Some(0))
        .x(Some(x_pos))
        .y(Some(0));
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
    if log_enabled!(log::Level::Debug) {
        let old_config = connection
            .get_geometry(window_id)
            .expect("could not get geometry")
            .reply()
            .expect("could not read geometry reply");
        debug!(
            "Positioning window {}. old: {:?}, new: {:?}",
            window_id, old_config, window_config
        );
    }
    connection
        .configure_window(window_id, &window_config)
        .expect("couldn't configure window");
    connection.flush().expect("couldn't flush");
    connection.sync().expect("couldn't sync");
}
