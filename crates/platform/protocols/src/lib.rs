const _SHELL_PROTOCOL_SOURCE: &str = include_str!("../protocols/bande-shell-v1.xml");
const _CAPTURE_PROTOCOL_SOURCE: &str = include_str!("../protocols/bande-capture-v1.xml");
const _APPMENU_PROTOCOL_SOURCE: &str = include_str!("../protocols/kde-appmenu.xml");
const _GTK_SHELL_PROTOCOL_SOURCE: &str = include_str!("../protocols/gtk-shell.xml");

#[cfg(feature = "client")]
pub mod client {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::xdg::shell::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::xdg::shell::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/bande-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/bande-shell-v1.xml");

    pub use self::capture::bande_capture_v1;

    mod capture {
        use wayland_client;

        pub mod __interfaces {
            wayland_scanner::generate_interfaces!("protocols/bande-capture-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("protocols/bande-capture-v1.xml");
    }

    pub use self::appmenu::{org_kde_kwin_appmenu, org_kde_kwin_appmenu_manager};

    mod appmenu {
        use wayland_client;
        use wayland_client::protocol::*;

        pub mod __interfaces {
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocols/kde-appmenu.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("protocols/kde-appmenu.xml");
    }
}

#[cfg(feature = "server")]
pub mod server {
    use wayland_protocols::xdg::shell::server::*;
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_protocols::xdg::shell::server::__interfaces::*;
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/bande-shell-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/bande-shell-v1.xml");

    pub use self::capture::bande_capture_v1;

    mod capture {
        use wayland_server;

        pub mod __interfaces {
            wayland_scanner::generate_interfaces!("protocols/bande-capture-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_server_code!("protocols/bande-capture-v1.xml");
    }

    pub use self::appmenu::{org_kde_kwin_appmenu, org_kde_kwin_appmenu_manager};

    mod appmenu {
        use wayland_server;
        use wayland_server::protocol::*;

        pub mod __interfaces {
            use wayland_server::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocols/kde-appmenu.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_server_code!("protocols/kde-appmenu.xml");
    }

    pub use self::gtk_shell::{gtk_shell1, gtk_surface1};

    mod gtk_shell {
        use wayland_server;
        use wayland_server::protocol::*;

        pub mod __interfaces {
            use wayland_server::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
    }
}
