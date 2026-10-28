#[cfg(feature = "client")]
pub mod client {
    use wayland_client;

    pub mod __interfaces {
        wayland_scanner::generate_interfaces!("protocols/unbound-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/unbound-shell-v1.xml");
}

#[cfg(feature = "server")]
pub mod server {
    use wayland_server;

    pub mod __interfaces {
        wayland_scanner::generate_interfaces!("protocols/unbound-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/unbound-shell-v1.xml");
}
