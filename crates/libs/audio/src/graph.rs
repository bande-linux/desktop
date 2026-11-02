use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Cursor, ErrorKind};
use std::rc::Rc;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use pipewire as pw;
use pw::device::{Device, DeviceListener};
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::registry::{GlobalObject, RegistryRc};
use pw::spa;
use pw::types::ObjectType;
use spa::param::ParamType;
use spa::pod::deserialize::PodDeserializer;
use spa::pod::serialize::PodSerializer;
use spa::pod::{Object, Pod, Property, Value, ValueArray};
use spa::sys;
use spa::utils::dict::DictRef;

use crate::stream::{self, INTERNAL_PREFIX, Running};
use crate::{Direction, Error, Sense, level};

const DEFAULT_OUTPUT: &str = "default.audio.sink";
const DEFAULT_INPUT: &str = "default.audio.source";
const CONFIGURED_OUTPUT: &str = "default.configured.audio.sink";
const CONFIGURED_INPUT: &str = "default.configured.audio.source";

pub(crate) type Reply = mpsc::Sender<Result<(), Error>>;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Change {
    Volume(Option<f32>, Option<bool>),
    Balance(f32),
}

pub(crate) enum Command {
    SetDefault {
        direction: Direction,
        name: String,
        done: Reply,
    },
    SetLevel {
        id: u32,
        change: Change,
        done: Reply,
    },
    Play {
        samples: Arc<[f32]>,
        gain: f32,
        done: Reply,
    },
    Meter {
        id: u64,
        device: String,
        peak: Arc<AtomicU32>,
        done: Reply,
    },
    Plugged {
        device: u32,
        profile_device: i32,
    },
    FollowJacks,
    Stop(u64),
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Volumes {
    pub channels: Vec<f32>,
    pub positions: Vec<u32>,
    pub muted: bool,
}

struct Props {
    channels: Vec<f32>,
    positions: Option<Vec<u32>>,
    muted: Option<bool>,
}

impl Volumes {
    fn apply(&mut self, props: Props) {
        self.channels = props.channels;
        if let Some(positions) = props.positions {
            self.positions = positions;
        }
        if let Some(muted) = props.muted {
            self.muted = muted;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NodeState {
    pub name: String,
    pub description: String,
    pub direction: Option<Direction>,
    pub volumes: Volumes,
    pub app: Option<App>,
    pub sense: Option<Sense>,
    pub camera: bool,
    pub(crate) device: Option<(u32, i32)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct App {
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    pub generation: u64,
    pub connected: bool,
    pub nodes: HashMap<u32, NodeState>,
    pub links: HashMap<u32, (u32, u32)>,
    routes: HashMap<(u32, i32), Volumes>,
    ports: HashMap<(u32, i32), bool>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

impl State {
    pub(crate) fn levels<'a>(&'a self, node: &'a NodeState) -> &'a Volumes {
        node.device
            .and_then(|device| self.routes.get(&device))
            .unwrap_or(&node.volumes)
    }

    pub(crate) fn available(&self, node: &NodeState) -> bool {
        node.device
            .and_then(|device| self.ports.get(&device))
            .is_none_or(|available| *available)
    }
}

#[derive(Default)]
pub(crate) struct Shared {
    state: Mutex<State>,
    pub changed: Condvar,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        change(&mut state);
        state.generation += 1;
        drop(state);
        self.changed.notify_all();
    }
}

struct Route {
    index: i32,
    device: Option<i32>,
    devices: Vec<i32>,
    available: bool,
}

#[derive(Default)]
struct Routes {
    active: Vec<Route>,
    enumerated: HashMap<i32, Route>,
}

impl Routes {
    fn availability(&self) -> HashMap<i32, bool> {
        let mut ports: HashMap<i32, bool> = HashMap::new();
        for route in self.enumerated.values() {
            for device in &route.devices {
                let available = ports.entry(*device).or_insert(false);
                *available |= route.available;
            }
        }
        for route in &self.active {
            if let Some(device) = route.device {
                ports.insert(device, route.available);
            }
        }
        ports
    }

    fn active_index(&self, profile_device: i32) -> Option<i32> {
        self.active
            .iter()
            .find(|route| route.device == Some(profile_device))
            .map(|route| route.index)
    }
}

struct BoundDevice {
    proxy: Device,
    _listener: DeviceListener,
    routes: Rc<RefCell<Routes>>,
}

#[derive(Default)]
struct Bound {
    nodes: HashMap<u32, (Node, NodeListener)>,
    devices: HashMap<u32, BoundDevice>,
    metadata: Option<(u32, Metadata, MetadataListener)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Device(Direction),
    AppStream,
    Capture(Sense),
    Camera,
}

fn kind_of(media_class: Option<&str>) -> Option<Kind> {
    match media_class? {
        "Audio/Sink" => Some(Kind::Device(Direction::Output)),
        "Audio/Source" | "Audio/Source/Virtual" => Some(Kind::Device(Direction::Input)),
        "Stream/Output/Audio" => Some(Kind::AppStream),
        "Stream/Input/Audio" => Some(Kind::Capture(Sense::Microphone)),
        "Stream/Input/Video" => Some(Kind::Capture(Sense::Camera)),
        "Video/Source" => Some(Kind::Camera),
        _ => None,
    }
}

fn object(pod: &Pod) -> Option<Object> {
    match PodDeserializer::deserialize_any_from(pod.as_bytes()) {
        Ok((_, Value::Object(object))) => Some(object),
        _ => None,
    }
}

fn levels(object: &Object) -> Option<Props> {
    let mut channels = None;
    let mut positions = None;
    let mut muted = None;
    for property in &object.properties {
        match (property.key, &property.value) {
            (sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(volumes))) => {
                channels = Some(volumes.clone());
            }
            (sys::SPA_PROP_channelMap, Value::ValueArray(ValueArray::Id(map))) => {
                positions = Some(map.iter().map(|position| position.0).collect());
            }
            (sys::SPA_PROP_mute, Value::Bool(value)) => muted = Some(*value),
            _ => {}
        }
    }
    channels.map(|channels| Props {
        channels,
        positions,
        muted,
    })
}

fn route_levels(object: &Object) -> Option<Props> {
    object
        .properties
        .iter()
        .find(|property| property.key == sys::SPA_PARAM_ROUTE_props)
        .and_then(|property| match &property.value {
            Value::Object(props) => levels(props),
            _ => None,
        })
}

fn route(object: &Object) -> Option<Route> {
    let mut index = None;
    let mut device = None;
    let mut devices = Vec::new();
    let mut available = true;
    for property in &object.properties {
        match (property.key, &property.value) {
            (sys::SPA_PARAM_ROUTE_index, Value::Int(value)) => index = Some(*value),
            (sys::SPA_PARAM_ROUTE_device, Value::Int(value)) => device = Some(*value),
            (sys::SPA_PARAM_ROUTE_devices, Value::ValueArray(ValueArray::Int(listed))) => {
                devices.clone_from(listed);
            }
            (sys::SPA_PARAM_ROUTE_available, Value::Id(id)) => {
                available = id.0 != sys::SPA_PARAM_AVAILABILITY_no;
            }
            _ => {}
        }
    }
    Some(Route {
        index: index?,
        device,
        devices,
        available,
    })
}

fn default_name(value: Option<&str>) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(value?).ok()?;
    json.get("name")?.as_str().map(str::to_owned)
}

fn serialize(object: Object) -> Result<Vec<u8>, Error> {
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .map(|(cursor, _)| cursor.into_inner())
        .map_err(|error| Error::PipeWire(format!("{error:?}")))
}

struct Graph {
    shared: Arc<Shared>,
    registry: RegistryRc,
    bound: RefCell<Bound>,
    core: pw::core::CoreRc,
    running: RefCell<HashMap<u64, Running>>,
    next_effect: Cell<u64>,
    follow_jacks: Cell<bool>,
    commands: pw::channel::Sender<Command>,
}

impl Graph {
    fn add(self: &Rc<Self>, global: &GlobalObject<&DictRef>) {
        let props = global.props;
        let result = match global.type_ {
            ObjectType::Node
                if props
                    .and_then(|p| p.get("node.name"))
                    .is_some_and(|name| name.starts_with(INTERNAL_PREFIX)) =>
            {
                Ok(())
            }
            ObjectType::Node => match kind_of(props.and_then(|p| p.get("media.class"))) {
                Some(kind) => self.add_node(global, kind),
                None => Ok(()),
            },
            ObjectType::Link => {
                let node = |key| {
                    props
                        .and_then(|p| p.get(key))
                        .and_then(|id| id.parse().ok())
                };
                if let Some(link) = node("link.output.node").zip(node("link.input.node")) {
                    self.shared.update(|state| {
                        state.links.insert(global.id, link);
                    });
                }
                Ok(())
            }
            ObjectType::Device
                if props.and_then(|p| p.get("media.class")) == Some("Audio/Device") =>
            {
                self.add_device(global)
            }
            ObjectType::Metadata
                if props.and_then(|p| p.get("metadata.name")) == Some("default") =>
            {
                self.add_metadata(global)
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("cannot follow PipeWire object {}: {error}", global.id);
        }
    }

    fn add_node(&self, global: &GlobalObject<&DictRef>, kind: Kind) -> Result<(), pw::Error> {
        let id = global.id;
        let node: Node = self.registry.bind(global)?;
        let info_shared = Arc::clone(&self.shared);
        let param_shared = Arc::clone(&self.shared);
        let listener = node
            .add_listener_local()
            .info(move |info| {
                if !info.change_mask().contains(NodeChangeMask::PROPS) {
                    return;
                }
                let Some(props) = info.props() else {
                    return;
                };
                let device = props
                    .get("device.id")
                    .and_then(|device| device.parse().ok())
                    .zip(
                        props
                            .get("card.profile.device")
                            .and_then(|route| route.parse().ok()),
                    );
                let name = props.get("node.name").unwrap_or_default().to_owned();
                let description = props
                    .get("node.description")
                    .or_else(|| props.get("node.nick"))
                    .map_or_else(|| name.clone(), str::to_owned);
                let camera = kind == Kind::Camera
                    && (props.get("media.role") == Some("Camera")
                        || matches!(props.get("device.api"), Some("v4l2" | "libcamera")));
                let listed = matches!(kind, Kind::AppStream | Kind::Capture(_))
                    && props.get("node.link-group").is_none();
                let app = listed.then(|| App {
                    name: props
                        .get("application.name")
                        .or_else(|| props.get("application.process.binary"))
                        .unwrap_or(description.as_str())
                        .to_owned(),
                    icon: props
                        .get("application.icon-name")
                        .or_else(|| props.get("application.id"))
                        .or_else(|| props.get("pipewire.access.portal.app_id"))
                        .map(str::to_owned),
                });
                info_shared.update(|state| {
                    let node = state.nodes.entry(id).or_default();
                    node.direction = match kind {
                        Kind::Device(direction) => Some(direction),
                        Kind::AppStream | Kind::Capture(_) | Kind::Camera => None,
                    };
                    node.sense = match kind {
                        Kind::Capture(sense) => Some(sense),
                        _ => None,
                    };
                    node.camera = camera;
                    node.app = app;
                    node.device = device;
                    node.name = name;
                    node.description = description;
                });
            })
            .param(move |_, kind, _, _, pod| {
                if kind != ParamType::Props {
                    return;
                }
                let Some(props) = pod.and_then(object).as_ref().and_then(levels) else {
                    return;
                };
                param_shared.update(|state| {
                    state.nodes.entry(id).or_default().volumes.apply(props);
                });
            })
            .register();
        node.subscribe_params(&[ParamType::Props]);
        self.bound.borrow_mut().nodes.insert(id, (node, listener));
        Ok(())
    }

    fn add_device(&self, global: &GlobalObject<&DictRef>) -> Result<(), pw::Error> {
        let id = global.id;
        let proxy: Device = self.registry.bind(global)?;
        let routes = Rc::new(RefCell::new(Routes::default()));
        let seen = Rc::clone(&routes);
        let shared = Arc::clone(&self.shared);
        let commands = self.commands.clone();
        let listener = proxy
            .add_listener_local()
            .param(move |_, kind, index, _, pod| {
                let mut routes = seen.borrow_mut();
                if kind == ParamType::Route && index == 0 {
                    routes.active.clear();
                }
                let Some(object) = pod.and_then(object) else {
                    return;
                };
                let Some(route) = route(&object) else {
                    return;
                };
                let props = route_levels(&object);
                let profile_device = route.device;
                match kind {
                    ParamType::Route => routes.active.push(route),
                    ParamType::EnumRoute => {
                        routes.enumerated.insert(route.index, route);
                    }
                    _ => return,
                }
                let ports = routes.availability();
                let mut plugged = Vec::new();
                shared.update(|state| {
                    for (profile_device, available) in ports {
                        let was = state.ports.insert((id, profile_device), available);
                        if available && was == Some(false) {
                            plugged.push(profile_device);
                        }
                    }
                    if let (Some(props), Some(profile_device)) = (props, profile_device) {
                        state
                            .routes
                            .entry((id, profile_device))
                            .or_default()
                            .apply(props);
                    }
                });
                for profile_device in plugged {
                    let _ = commands.send(Command::Plugged {
                        device: id,
                        profile_device,
                    });
                }
            })
            .register();
        proxy.subscribe_params(&[ParamType::EnumRoute, ParamType::Route]);
        self.bound.borrow_mut().devices.insert(
            global.id,
            BoundDevice {
                proxy,
                _listener: listener,
                routes,
            },
        );
        Ok(())
    }

    fn add_metadata(&self, global: &GlobalObject<&DictRef>) -> Result<(), pw::Error> {
        let metadata: Metadata = self.registry.bind(global)?;
        let shared = Arc::clone(&self.shared);
        let listener = metadata
            .add_listener_local()
            .property(move |subject, key, _, value| {
                if subject == 0 {
                    match key {
                        Some(DEFAULT_OUTPUT) => {
                            shared.update(|state| state.default_output = default_name(value));
                        }
                        Some(DEFAULT_INPUT) => {
                            shared.update(|state| state.default_input = default_name(value));
                        }
                        _ => {}
                    }
                }
                0
            })
            .register();
        self.bound.borrow_mut().metadata = Some((global.id, metadata, listener));
        Ok(())
    }

    fn remove(&self, id: u32) {
        let mut bound = self.bound.borrow_mut();
        if bound.devices.remove(&id).is_some() {
            self.shared.update(|state| {
                state.routes.retain(|(device, _), _| *device != id);
                state.ports.retain(|(device, _), _| *device != id);
            });
        }
        if bound
            .metadata
            .as_ref()
            .is_some_and(|(metadata, ..)| *metadata == id)
        {
            bound.metadata = None;
            self.shared.update(|state| {
                state.default_output = None;
                state.default_input = None;
            });
        }
        if self.shared.lock().links.contains_key(&id) {
            self.shared.update(|state| {
                state.links.remove(&id);
            });
        }
        if bound.nodes.remove(&id).is_some() {
            self.shared.update(|state| {
                state.nodes.remove(&id);
            });
        }
    }

    fn play(&self, samples: Arc<[f32]>, gain: f32) -> Result<(), Error> {
        let id = self.next_effect.get();
        self.next_effect.set(id + 1);
        let commands = self.commands.clone();
        let running = stream::play(&self.core, samples, gain, move || {
            let _ = commands.send(Command::Stop(id));
        })?;
        self.running.borrow_mut().insert(id, running);
        Ok(())
    }

    fn meter(&self, id: u64, device: &str, peak: Arc<AtomicU32>) -> Result<(), Error> {
        let running = stream::meter(&self.core, device, peak)?;
        self.running.borrow_mut().insert(id, running);
        Ok(())
    }

    fn set_default(&self, direction: Direction, name: &str) -> Result<(), Error> {
        let bound = self.bound.borrow();
        let (_, metadata, _) = bound.metadata.as_ref().ok_or(Error::NoSessionManager)?;
        let key = match direction {
            Direction::Output => CONFIGURED_OUTPUT,
            Direction::Input => CONFIGURED_INPUT,
        };
        let value = serde_json::json!({ "name": name }).to_string();
        metadata.set_property(0, key, Some("Spa:String:JSON"), Some(&value));
        Ok(())
    }

    fn plugged(&self, device: u32, profile_device: i32) -> Result<(), Error> {
        if !self.follow_jacks.get() {
            return Ok(());
        }
        let jack = {
            let state = self.shared.lock();
            state
                .nodes
                .values()
                .find(|node| node.device == Some((device, profile_device)))
                .and_then(|node| Some((node.direction?, node.name.clone())))
        };
        match jack {
            Some((direction, name)) => self.set_default(direction, &name),
            None => Ok(()),
        }
    }

    fn set_level(&self, id: u32, change: Change) -> Result<(), Error> {
        let (node, current) = {
            let state = self.shared.lock();
            let node = state.nodes.get(&id).cloned().ok_or(Error::NoSuchDevice)?;
            let current = state.levels(&node).clone();
            (node, current)
        };
        let channels = match change {
            Change::Volume(Some(volume), _) => level::scaled(&current.channels, volume),
            Change::Volume(None, _) => current.channels,
            Change::Balance(balance) => {
                level::balanced(&current, balance).ok_or(Error::NoSuchDevice)?
            }
        };
        let muted = match change {
            Change::Volume(_, Some(muted)) => muted,
            Change::Volume(_, None) | Change::Balance(_) => current.muted,
        };
        let props = Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Props,
            properties: vec![
                Property::new(
                    sys::SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(channels)),
                ),
                Property::new(sys::SPA_PROP_mute, Value::Bool(muted)),
            ],
        };
        let bound = self.bound.borrow();
        let route = node.device.and_then(|(device, profile_device)| {
            let device = bound.devices.get(&device)?;
            let index = device.routes.borrow().active_index(profile_device)?;
            Some((device, index, profile_device))
        });
        match route {
            Some((device, index, profile_device)) => {
                let bytes = serialize(Object {
                    type_: sys::SPA_TYPE_OBJECT_ParamRoute,
                    id: sys::SPA_PARAM_Route,
                    properties: vec![
                        Property::new(sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
                        Property::new(sys::SPA_PARAM_ROUTE_device, Value::Int(profile_device)),
                        Property::new(sys::SPA_PARAM_ROUTE_props, Value::Object(props)),
                        Property::new(sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
                    ],
                })?;
                let pod = Pod::from_bytes(&bytes).ok_or(Error::PipeWire("invalid route".into()))?;
                device.proxy.set_param(ParamType::Route, 0, pod);
            }
            None => {
                let (proxy, _) = bound.nodes.get(&id).ok_or(Error::NoSuchDevice)?;
                let bytes = serialize(props)?;
                let pod = Pod::from_bytes(&bytes).ok_or(Error::PipeWire("invalid props".into()))?;
                proxy.set_param(ParamType::Props, 0, pod);
            }
        }
        Ok(())
    }
}

pub(crate) fn run(
    shared: Arc<Shared>,
    commands: pw::channel::Receiver<Command>,
    sender: pw::channel::Sender<Command>,
    ready: mpsc::Sender<Result<(), Error>>,
) {
    if let Err(error) = serve(&shared, commands, sender, &ready) {
        let _ = ready.send(Err(error));
    }
    shared.update(|state| {
        state.connected = false;
        state.nodes.clear();
    });
}

fn serve(
    shared: &Arc<Shared>,
    commands: pw::channel::Receiver<Command>,
    sender: pw::channel::Sender<Command>,
    ready: &mpsc::Sender<Result<(), Error>>,
) -> Result<(), Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let graph = Rc::new(Graph {
        shared: Arc::clone(shared),
        registry: core.get_registry_rc()?,
        bound: RefCell::default(),
        core: core.clone(),
        running: RefCell::default(),
        next_effect: Cell::new(1 << 32),
        follow_jacks: Cell::new(false),
        commands: sender,
    });

    let added = Rc::clone(&graph);
    let removed = Rc::clone(&graph);
    let _registry = graph
        .registry
        .add_listener_local()
        .global(move |global| added.add(global))
        .global_remove(move |id| removed.remove(id))
        .register();

    let pending: Rc<RefCell<Vec<(spa::utils::result::AsyncSeq, Reply)>>> = Rc::default();
    let (first, second) = (core.sync(0)?, Rc::new(RefCell::new(None)));
    let done_core = core.clone();
    let done_pending = Rc::clone(&pending);
    let done_shared = Arc::clone(shared);
    let done_ready = ready.clone();
    let done_second = Rc::clone(&second);
    let lost_loop = main_loop.clone();
    let _core = core
        .add_listener_local()
        .done(move |id, seq| {
            if id != pw::core::PW_ID_CORE {
                return;
            }
            if seq == first {
                match done_core.sync(0) {
                    Ok(next) => *done_second.borrow_mut() = Some(next),
                    Err(error) => {
                        let _ = done_ready.send(Err(error.into()));
                    }
                }
            } else if Some(seq) == *done_second.borrow() {
                done_shared.update(|state| state.connected = true);
                let _ = done_ready.send(Ok(()));
            }
            let mut pending = done_pending.borrow_mut();
            if let Some(position) = pending.iter().position(|(waiting, _)| *waiting == seq) {
                let (_, reply) = pending.remove(position);
                let _ = reply.send(Ok(()));
            }
        })
        .error(move |id, _, result, message| {
            let lost = std::io::Error::from_raw_os_error(-result).kind() == ErrorKind::BrokenPipe;
            if id == pw::core::PW_ID_CORE && lost {
                tracing::warn!("PipeWire connection lost: {message}");
                lost_loop.quit();
            } else {
                tracing::debug!("PipeWire error on {id} ({result}): {message}");
            }
        })
        .register();

    let command_loop = main_loop.clone();
    let command_core = core.clone();
    let command_graph = Rc::clone(&graph);
    let _commands = commands.attach(main_loop.loop_(), move |command| {
        let (outcome, done) = match command {
            Command::Quit => {
                command_loop.quit();
                return;
            }
            Command::SetDefault {
                direction,
                name,
                done,
            } => (command_graph.set_default(direction, &name), done),
            Command::SetLevel { id, change, done } => (command_graph.set_level(id, change), done),
            Command::Play {
                samples,
                gain,
                done,
            } => (command_graph.play(samples, gain), done),
            Command::Meter {
                id,
                device,
                peak,
                done,
            } => (command_graph.meter(id, &device, peak), done),
            Command::Plugged {
                device,
                profile_device,
            } => {
                if let Err(error) = command_graph.plugged(device, profile_device) {
                    tracing::warn!("cannot switch to the plugged-in device: {error}");
                }
                return;
            }
            Command::FollowJacks => {
                command_graph.follow_jacks.set(true);
                return;
            }
            Command::Stop(id) => {
                command_graph.running.borrow_mut().remove(&id);
                return;
            }
        };
        match outcome.and_then(|()| command_core.sync(0).map_err(Error::from)) {
            Ok(seq) => pending.borrow_mut().push((seq, done)),
            Err(error) => {
                let _ = done.send(Err(error));
            }
        }
    });

    main_loop.run();
    Ok(())
}
