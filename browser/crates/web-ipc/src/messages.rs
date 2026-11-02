use url::Url;

use crate::wire::{wire_enum, wire_struct};

wire_struct! {
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Size {
        pub width: f32,
        pub height: f32,
    }
}

wire_enum! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Destination {
        Document = 0,
        Style = 1,
        Image = 2,
        Font = 3,
    }
}

wire_enum! {
    #[derive(Debug, Clone)]
    pub enum ToRenderer {
        Commit { load: u64, url: Url, charset: Option<String>, viewport: Size } = 0,
        Resize { viewport: Size } = 1,
        Hover { load: u64, x: f32, y: f32 } = 2,
        Fragment { load: u64, name: String } = 3,
        Stop = 4,
        Network = 5,
    }
}

wire_enum! {
    #[derive(Debug, Clone)]
    pub enum FromRenderer {
        Font { id: u32, index: u32 } = 0,
        Image { load: u64, id: u32, width: u32, height: u32, natural_width: u32, natural_height: u32 } = 1,
        Frame { load: u64, title: Option<String>, fragment: Option<f32> } = 2,
        Relaid { load: u64 } = 3,
        Scroll { load: u64, y: f32 } = 4,
    }
}

wire_enum! {
    #[derive(Debug, Clone)]
    pub enum ToNetwork {
        Fetch { id: u64, url: Url, destination: Destination } = 0,
        Cancel { id: u64 } = 1,
    }
}

wire_enum! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Verdict {
        Continue = 0,
        Cancel = 1,
        Redirect { url: Url } = 2,
    }
}

wire_enum! {
    #[derive(Debug, Clone)]
    pub enum NetworkControl {
        Fetch { id: u64, session: u32, url: Url, destination: Destination } = 0,
        Cancel { id: u64 } = 1,
        Connect { client: u32, session: u32, site: String } = 2,
        EndSession { session: u32 } = 3,
        Intercept { enabled: bool } = 4,
        Resume { request: u64, verdict: Verdict } = 5,
        Clear { session: u32, since: u64, cookies: bool, cache: bool, sites: Option<Vec<String>> } = 6,
        SiteData { request: u64, session: u32 } = 7,
    }
}

wire_enum! {
    #[derive(Debug, Clone)]
    pub enum FromNetwork {
        Fetched {
            id: u64,
            url: Url,
            status: u16,
            content_type: Option<String>,
            html: bool,
            charset: Option<String>,
        } = 0,
        Failed { id: u64, message: String } = 1,
        Paused { request: u64, client: u32, id: u64, url: Url, destination: Destination } = 2,
        SiteData { request: u64 } = 3,
    }
}
