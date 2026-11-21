use bande_discovery::nearby::Service;

pub const KINDS: [&str; 2] = [IPP, IPPS];
const IPP: &str = "_ipp._tcp";
const IPPS: &str = "_ipps._tcp";
const RESOURCE: &str = "ipp/print";

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

pub fn printers(services: Vec<Service>) -> Vec<Found> {
    let mut printers: Vec<Found> = Vec::new();
    for service in services {
        if !printers
            .iter()
            .any(|printer| printer.instance == service.name)
        {
            let value = |key: &str| service.value(key).unwrap_or_default();
            printers.push(Found {
                instance: service.name.clone(),
                model: value("ty"),
                location: value("note"),
                uuid: value("uuid"),
                plain: None,
                secure: None,
            });
        }
        let Some(printer) = printers
            .iter_mut()
            .find(|printer| printer.instance == service.name)
        else {
            continue;
        };
        let resource = service
            .value("rp")
            .filter(|resource| !resource.is_empty())
            .unwrap_or_else(|| RESOURCE.to_owned());
        let url = |scheme: &str| {
            format!(
                "{scheme}://{}:{}/{}",
                service.address,
                service.port,
                resource.trim_start_matches('/')
            )
        };
        match service.kind.as_str() {
            IPPS => printer.secure = Some(url("ipps")),
            _ => printer.plain = Some(url("ipp")),
        }
    }
    printers.sort_by_key(|printer| printer.instance.to_lowercase());
    printers
}
