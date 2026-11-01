#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::collections::HashMap;

use bande_test_bus::SessionBus;
use xdg_desktop_portal_bande::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::export::serde::Serialize;
use zbus::zvariant::{DynamicType, OwnedValue, Value};

pub type Results = HashMap<String, OwnedValue>;
pub type Options<'a> = HashMap<&'a str, Value<'a>>;

pub struct Portal {
    pub service: Connection,
    pub client: Connection,
    pub bus: SessionBus,
}

impl Portal {
    pub fn start(serve: impl FnOnce(&Connection)) -> Self {
        let bus = SessionBus::start().expect("dbus-daemon");
        let service = bus.connect().expect("portal connection");
        serve(&service);
        service.request_name(BUS_NAME).expect("portal name");
        let client = bus.connect().expect("frontend connection");
        client
            .request_name(xdg_desktop_portal_bande::FRONTEND)
            .expect("frontend name");
        Self {
            service,
            client,
            bus,
        }
    }

    pub fn call<B: Serialize + DynamicType>(
        &self,
        interface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::message::Message> {
        self.client
            .call_method(Some(BUS_NAME), OBJECT_PATH, Some(interface), method, body)
    }

    pub fn request<B: Serialize + DynamicType>(
        &self,
        interface: &str,
        method: &str,
        body: &B,
    ) -> (u32, Results) {
        self.call(interface, method, body)
            .expect("call")
            .body()
            .deserialize()
            .expect("(ua{sv})")
    }

    pub fn close(&self, path: &str, interface: &str) {
        self.client
            .call_method(Some(BUS_NAME), path, Some(interface), "Close", &())
            .expect("Close");
    }
}

pub fn get<T: TryFrom<OwnedValue>>(results: &Results, key: &str) -> T {
    results
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
        .unwrap_or_else(|| panic!("{key} missing or mistyped in {results:?}"))
}
