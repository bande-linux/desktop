use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use pipewire as pw;
use pw::properties::PropertiesBox;
use pw::spa;
use pw::stream::{StreamFlags, StreamListener, StreamRc};
use spa::param::audio::{AudioFormat, AudioInfoRaw};
use spa::pod::serialize::PodSerializer;
use spa::pod::{Object, Pod, Value};
use spa::sys;

use crate::Error;

pub const INTERNAL_PREFIX: &str = "bande-";
pub const EFFECT_RATE: u32 = 48_000;
const SAMPLE_BYTES: usize = size_of::<f32>();

pub(crate) struct Running {
    _stream: StreamRc,
    _listener: Box<dyn std::any::Any>,
}

fn format(rate: Option<u32>, channels: Option<u32>) -> Result<Vec<u8>, Error> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::F32LE);
    if let Some(rate) = rate {
        info.set_rate(rate);
    }
    if let Some(channels) = channels {
        info.set_channels(channels);
        let mut position = [0; spa::param::audio::MAX_CHANNELS];
        position[0] = sys::SPA_AUDIO_CHANNEL_MONO;
        info.set_position(position);
    }
    PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: sys::SPA_TYPE_OBJECT_Format,
            id: sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|error| Error::PipeWire(format!("{error:?}")))
}

fn properties(entries: &[(&str, &str)]) -> PropertiesBox {
    let mut properties = PropertiesBox::new();
    for (key, value) in entries {
        properties.insert(*key, *value);
    }
    properties
}

pub(crate) fn play(
    core: &pw::core::CoreRc,
    samples: Arc<[f32]>,
    gain: f32,
    finished: impl Fn() + 'static,
) -> Result<Running, Error> {
    let stream = StreamRc::new(
        core.clone(),
        "bande-sound-effect",
        properties(&[
            (*pw::keys::MEDIA_TYPE, "Audio"),
            (*pw::keys::MEDIA_CATEGORY, "Playback"),
            (*pw::keys::MEDIA_ROLE, "Event"),
            (*pw::keys::NODE_NAME, "bande-sound-effect"),
            (*pw::keys::APP_NAME, "Bande"),
            (*pw::keys::NODE_LATENCY, "256/48000"),
        ]),
    )?;
    let listener: StreamListener<usize> = stream
        .add_local_listener_with_user_data(0_usize)
        .process(move |stream, played| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let written = data.data().map_or(0, |bytes| {
                let left = samples.get(*played..).unwrap_or_default();
                let frames = (bytes.len() / SAMPLE_BYTES).min(left.len());
                for (target, sample) in bytes
                    .as_chunks_mut::<SAMPLE_BYTES>()
                    .0
                    .iter_mut()
                    .zip(&left[..frames])
                {
                    *target = (sample * gain).to_le_bytes();
                }
                frames
            });
            *played += written;
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = SAMPLE_BYTES as i32;
            *chunk.size_mut() = (written * SAMPLE_BYTES) as u32;
            if *played >= samples.len() {
                drop(buffer);
                let _ = stream.flush(true);
            }
        })
        .drained(move |_, _| finished())
        .register()?;
    let bytes = format(Some(EFFECT_RATE), Some(1))?;
    let pod = Pod::from_bytes(&bytes).ok_or(Error::PipeWire("invalid format".into()))?;
    stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut [pod],
    )?;
    Ok(Running {
        _stream: stream,
        _listener: Box::new(listener),
    })
}

pub(crate) fn meter(
    core: &pw::core::CoreRc,
    device: &str,
    peak: Arc<AtomicU32>,
) -> Result<Running, Error> {
    let stream = StreamRc::new(
        core.clone(),
        "bande-level-meter",
        properties(&[
            (*pw::keys::MEDIA_TYPE, "Audio"),
            (*pw::keys::MEDIA_CATEGORY, "Capture"),
            (*pw::keys::MEDIA_ROLE, "DSP"),
            (*pw::keys::NODE_NAME, "bande-level-meter"),
            (*pw::keys::APP_NAME, "Bande"),
            ("target.object", device),
            (*pw::keys::NODE_LATENCY, "1024/48000"),
        ]),
    )?;
    let listener: StreamListener<()> = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, ()| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let loudest = bytes
                .get(..size)
                .unwrap_or_default()
                .as_chunks::<SAMPLE_BYTES>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(*sample))
                .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
            peak.fetch_max(loudest.to_bits(), Ordering::Relaxed);
        })
        .register()?;
    let bytes = format(None, None)?;
    let pod = Pod::from_bytes(&bytes).ok_or(Error::PipeWire("invalid format".into()))?;
    stream.connect(
        spa::utils::Direction::Input,
        None,
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut [pod],
    )?;
    Ok(Running {
        _stream: stream,
        _listener: Box::new(listener),
    })
}
