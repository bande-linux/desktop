use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
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

pub(crate) enum Command {
    SetDefault {
        direction: Direction,
        name: String,
        done: Reply,
    },
    SetLevel {
        id: u32,
        volume: Option<f32>,
        muted: Option<bool>,
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
    Stop(u64),
    Quit,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NodeState {
    pub name: String,
    pub description: String,
    pub direction: Option<Direction>,
    pub channels: Vec<f32>,
    pub muted: bool,
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
    routes: HashMap<(u32, i32), (Vec<f32>, bool)>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

impl State {
    pub(crate) fn levels<'a>(&'a self, node: &'a NodeState) -> (&'a [f32], bool) {
        node.device
            .and_then(|device| self.routes.get(&device))
            .map_or(
                (node.channels.as_slice(), node.muted),
                |(channels, muted)| (channels.as_slice(), *muted),
            )
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

struct BoundDevice {
    proxy: Device,
    _listener: DeviceListener,
    routes: Rc<RefCell<Vec<(i32, i32)>>>,
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

fn levels(object: &Object) -> Option<(Vec<f32>, Option<bool>)> {
    let mut channels = None;
    let mut muted = None;
    for property in &object.properties {
        match (property.key, &property.value) {
            (sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(volumes))) => {
                channels = Some(volumes.clone());
            }
            (sys::SPA_PROP_mute, Value::Bool(value)) => muted = Some(*value),
            _ => {}
        }
    }
    channels.map(|channels| (channels, muted))
}

fn route_levels(object: &Object) -> Option<(Vec<f32>, Option<bool>)> {
    object
        .properties
        .iter()
        .find(|property| property.key == sys::SPA_PARAM_ROUTE_props)
        .and_then(|property| match &property.value {
            Value::Object(props) => levels(props),
            _ => None,
        })
}

fn route(object: &Object) -> Option<(i32, i32)> {
    let int = |key| {
        object
            .properties
            .iter()
            .find(|property| property.key == key)
            .and_then(|property| match property.value {
                Value::Int(value) => Some(value),
                _ => None,
            })
    };
    Some((
        int(sys::SPA_PARAM_ROUTE_index)?,
        int(sys::SPA_PARAM_ROUTE_device)?,
    ))
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
    next_effect: std::cell::Cell<u64>,
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
                let app = matches!(kind, Kind::AppStream | Kind::Capture(_)).then(|| App {
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
                let Some((channels, muted)) = pod.and_then(object).as_ref().and_then(levels) else {
                    return;
                };
                param_shared.update(|state| {
                    let node = state.nodes.entry(id).or_default();
                    node.channels = channels;
                    if let Some(muted) = muted {
                        node.muted = muted;
                    }
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
        let routes = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&routes);
        let shared = Arc::clone(&self.shared);
        let listener = proxy
            .add_listener_local()
            .param(move |_, kind, index, _, pod| {
                if kind != ParamType::Route {
                    return;
                }
                let mut routes = seen.borrow_mut();
                if index == 0 {
                    routes.clear();
                }
                let Some(object) = pod.and_then(object) else {
                    return;
                };
                let Some(route) = route(&object) else {
                    return;
                };
                routes.push(route);
                if let Some((channels, muted)) = route_levels(&object) {
                    let key = (id, route.1);
                    shared.update(|state| {
                        let muted = muted
                            .or_else(|| state.routes.get(&key).map(|(_, muted)| *muted))
                            .unwrap_or(false);
                        state.routes.insert(key, (channels, muted));
                    });
                }
            })
            .register();
        proxy.subscribe_params(&[ParamType::Route]);
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
            self.shared
                .update(|state| state.routes.retain(|(device, _), _| *device != id));
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

    fn set_level(&self, id: u32, volume: Option<f32>, muted: Option<bool>) -> Result<(), Error> {
        let (node, current, current_muted) = {
            let state = self.shared.lock();
            let node = state.nodes.get(&id).cloned().ok_or(Error::NoSuchDevice)?;
            let (channels, muted) = state.levels(&node);
            let current = channels.to_vec();
            (node, current, muted)
        };
        let channels = match volume {
            Some(volume) => level::scaled(&current, volume),
            None => current,
        };
        let props = Object {
            type_: sys::SPA_TYPE_OBJECT_Props,
            id: sys::SPA_PARAM_Props,
            properties: vec![
                Property::new(
                    sys::SPA_PROP_channelVolumes,
                    Value::ValueArray(ValueArray::Float(channels)),
                ),
                Property::new(
                    sys::SPA_PROP_mute,
                    Value::Bool(muted.unwrap_or(current_muted)),
                ),
            ],
        };
        let bound = self.bound.borrow();
        let route = node.device.and_then(|(device, profile_device)| {
            let device = bound.devices.get(&device)?;
            let index = device
                .routes
                .borrow()
                .iter()
                .find(|(_, route_device)| *route_device == profile_device)
                .map(|(index, _)| *index)?;
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
        next_effect: std::cell::Cell::new(1 << 32),
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
            if id == pw::core::PW_ID_CORE {
                tracing::warn!("PipeWire connection lost ({result}): {message}");
                lost_loop.quit();
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
            Command::SetLevel {
                id,
                volume,
                muted,
                done,
            } => (command_graph.set_level(id, volume, muted), done),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(device: Option<(u32, i32)>, channels: Vec<f32>) -> NodeState {
        NodeState {
            name: "sink".to_owned(),
            channels,
            device,
            ..NodeState::default()
        }
    }

    #[test]
    fn a_device_sink_takes_its_level_from_the_active_route_not_its_props() {
        let mut state = State::default();
        state.routes.insert((43, 1), (vec![0.064, 0.064], true));
        let speaker = sink(Some((43, 1)), vec![1.0, 1.0]);

        assert_eq!(state.levels(&speaker), (&[0.064, 0.064][..], true));
    }

    #[test]
    fn a_virtual_sink_keeps_the_level_of_its_props() {
        let mut state = State::default();
        state.routes.insert((43, 1), (vec![0.064, 0.064], true));
        let virtual_sink = sink(None, vec![0.5, 0.5]);

        assert_eq!(state.levels(&virtual_sink), (&[0.5, 0.5][..], false));
    }
}
