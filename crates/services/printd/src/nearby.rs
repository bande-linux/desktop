use bande_discovery::nearby::Service;

pub const KINDS: [&str; 4] = [IPP, IPPS, USCAN, USCANS];
const IPP: &str = "_ipp._tcp";
const IPPS: &str = "_ipps._tcp";
const USCAN: &str = "_uscan._tcp";
const USCANS: &str = "_uscans._tcp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub instance: String,
    pub model: String,
    pub location: String,
    pub uuid: String,
    pub plain: Option<String>,
    pub secure: Option<String>,
}

impl Found {
    pub fn id(&self) -> String {
        match self.uuid.is_empty() {
            true => self.instance.clone(),
            false => self.uuid.to_lowercase(),
        }
    }
}

struct Protocol {
    plain: (&'static str, &'static str),
    secure: (&'static str, &'static str),
    resource_key: &'static str,
    resource: &'static str,
}

const PRINTING: Protocol = Protocol {
    plain: (IPP, "ipp"),
    secure: (IPPS, "ipps"),
    resource_key: "rp",
    resource: "ipp/print",
};

const SCANNING: Protocol = Protocol {
    plain: (USCAN, "http"),
    secure: (USCANS, "https"),
    resource_key: "rs",
    resource: "eSCL",
};

pub fn printers(services: &[Service]) -> Vec<Found> {
    devices(services, &PRINTING)
}

pub fn scanners(services: &[Service]) -> Vec<Found> {
    devices(services, &SCANNING)
}

fn devices(services: &[Service], protocol: &Protocol) -> Vec<Found> {
    let mut devices: Vec<Found> = Vec::new();
    for service in services {
        let secure = match service.kind.as_str() {
            kind if kind == protocol.plain.0 => false,
            kind if kind == protocol.secure.0 => true,
            _ => continue,
        };
        if !devices.iter().any(|device| device.instance == service.name) {
            let value = |key: &str| service.value(key).unwrap_or_default();
            devices.push(Found {
                instance: service.name.clone(),
                model: value("ty"),
                location: value("note"),
                uuid: value("uuid"),
                plain: None,
                secure: None,
            });
        }
        let Some(device) = devices
            .iter_mut()
            .find(|device| device.instance == service.name)
        else {
            continue;
        };
        let resource = service
            .value(protocol.resource_key)
            .filter(|resource| !resource.is_empty())
            .unwrap_or_else(|| protocol.resource.to_owned());
        let host = match service.host.is_empty() {
            true => &service.address,
            false => &service.host,
        };
        let url = |scheme: &str| {
            format!(
                "{scheme}://{host}:{}/{}",
                service.port,
                resource.trim_matches('/')
            )
        };
        match secure {
            true => device.secure = Some(url(protocol.secure.1)),
            false => device.plain = Some(url(protocol.plain.1)),
        }
    }
    devices.sort_by_key(|device| device.instance.to_lowercase());
    devices
}
