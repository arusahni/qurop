use log::{debug, info, trace};
use std::{sync::mpsc, thread::sleep, time::Duration};
use x11rb::{
    connection::Connection,
    properties::WmClass,
    protocol::{xproto::*, Event},
};

x11rb::atom_manager! {
    pub(crate) Atoms:
    AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_CLASS,
        _NET_WM_NAME,
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

/// Get the ID of the managed window
pub(crate) fn get_qurop_window_id(
    connection: &x11rb::rust_connection::RustConnection,
    root: u32,
) -> Option<u32> {
    let response = connection
        .query_tree(root)
        .expect("unable to query tree")
        .reply()
        .unwrap();

    response.children.into_iter().find(|child| {
        if let Some(class_name) = get_window_class(connection, *child) {
            trace!("Found class {class_name} ({child})");
            class_name == "qurop"
        } else {
            false
        }
    })
}

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

pub(crate) fn get_window_class(
    conn: &x11rb::rust_connection::RustConnection,
    window_id: u32,
) -> Option<String> {
    trace!("Getting window class for {window_id}");
    let class = WmClass::get(conn, window_id).expect("wmclass error");
    match class.reply() {
        Ok(wm_class) => Some(String::from(std::str::from_utf8(wm_class.class()).unwrap())),
        Err(_) => None,
    }
}

/// Handle window focus issues
pub(crate) fn handle_window(tx: mpsc::Sender<String>) {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let root = screen.root;
    let atoms = Atoms::new(&connection).unwrap().reply().unwrap();
    let active_atom = atoms._NET_ACTIVE_WINDOW;
    debug!(
        "window handler: Root {:?} | active_atom {}",
        root, active_atom
    );
    let mut window_id: Option<u32> = None;
    loop {
        window_id = get_qurop_window_id(&connection, root);
        if window_id.is_some() {
            break;
        }
        sleep(Duration::from_millis(100));
    }
    if let Some(window) = window_id {
        let active_class =
            get_window_class(&connection, window).unwrap_or_else(|| "unknown".into());
        let active_name = get_window_name(&connection, window, atoms);
        debug!("Active window: {window} ({active_name} | {active_class})");
        let event_sub = ChangeWindowAttributesAux::default().event_mask(EventMask::PROPERTY_CHANGE);
        connection
            .change_window_attributes(root, &event_sub)
            .expect("couldn't watch attributes");
        info!("starting waiting for events with window {}", window);
        connection.flush().unwrap();
        loop {
            let event = connection
                .wait_for_event()
                .expect("could not wait for xserver events");

            if let Event::PropertyNotify(e) = event {
                trace!("Property notify event for {}", e.atom);
                if e.atom == active_atom {
                    if let Ok(active_window) = get_active_window(&connection, screen, active_atom) {
                        if active_window != window {
                            debug!("sending unmap request: {} != {}", active_window, window);
                            tx.send(format!("unmap:{window}"))
                                .expect("couldn't send unmap command");
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn unmap_qurop_window() {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id =
        get_qurop_window_id(&connection, screen.root).expect("could not find window");
    info!("Unmapping qurop window: {qurop_window_id}");
    connection
        .unmap_window(qurop_window_id)
        .expect("could not unmap window");
    connection.flush().expect("couldn't flush");
}

pub(crate) fn unmap_window(window_id: u32) {
    let (connection, _num) = x11rb::connect(None).expect("x11 connection missing");
    info!("Unmapping window: {window_id}");
    connection
        .unmap_window(window_id)
        .expect("could not unmap window");
    connection.flush().expect("couldn't flush");
}

pub(crate) fn map_qurop_window() {
    let (connection, num) = x11rb::connect(None).expect("x11 connection missing");
    let screen = &connection.setup().roots[num];
    let qurop_window_id =
        get_qurop_window_id(&connection, screen.root).expect("could not find window");
    info!("Mapping qurop window: {qurop_window_id}");
    connection
        .map_window(qurop_window_id)
        .expect("could not map window");
    connection.flush().expect("couldn't flush");
}
