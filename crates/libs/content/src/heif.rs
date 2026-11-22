use std::ffi::{c_char, c_int, c_void};
use std::ptr;

use image::RgbaImage;
use libloading::Library;

use crate::decode::{Failure, Kind, allowed};

const LIBRARY: &str = "libheif.so.1";
const OK: c_int = 0;
const PLUGIN_LOADING_ERROR: c_int = 11;
const COLORSPACE_RGB: c_int = 1;
const CHROMA_INTERLEAVED_RGBA: c_int = 11;
const CHANNEL_INTERLEAVED: c_int = 10;
const COMPRESSION_HEVC: c_int = 1;
const COMPRESSION_AV1: c_int = 4;
const HDR_TO_8BIT_VERSION: u8 = 2;

#[repr(C)]
struct HeifError {
    code: c_int,
    _subcode: c_int,
    _message: *const c_char,
}

#[repr(C)]
struct DecodingOptions {
    version: u8,
    _ignore_transformations: u8,
    _start_progress: *const c_void,
    _on_progress: *const c_void,
    _end_progress: *const c_void,
    _progress_user_data: *mut c_void,
    convert_hdr_to_8bit: u8,
}

type Init = unsafe extern "C" fn(*const c_void) -> HeifError;
type Deinit = unsafe extern "C" fn();
type HaveDecoder = unsafe extern "C" fn(c_int) -> c_int;
type ContextAlloc = unsafe extern "C" fn() -> *mut c_void;
type Release = unsafe extern "C" fn(*mut c_void);
type ReadFromMemory =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *const c_void) -> HeifError;
type PrimaryHandle = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> HeifError;
type HandleSide = unsafe extern "C" fn(*const c_void) -> c_int;
type OptionsAlloc = unsafe extern "C" fn() -> *mut DecodingOptions;
type OptionsFree = unsafe extern "C" fn(*mut DecodingOptions);
type DecodeImage = unsafe extern "C" fn(
    *const c_void,
    *mut *mut c_void,
    c_int,
    c_int,
    *const DecodingOptions,
) -> HeifError;
type ImageSide = unsafe extern "C" fn(*const c_void, c_int) -> c_int;
type Plane = unsafe extern "C" fn(*const c_void, c_int, *mut usize) -> *const u8;

struct Api {
    deinit: Deinit,
    have_decoder: HaveDecoder,
    context_alloc: ContextAlloc,
    context_free: Release,
    read_from_memory: ReadFromMemory,
    primary_handle: PrimaryHandle,
    handle_width: HandleSide,
    handle_height: HandleSide,
    handle_release: Release,
    options_alloc: OptionsAlloc,
    options_free: OptionsFree,
    decode_image: DecodeImage,
    image_width: ImageSide,
    image_height: ImageSide,
    plane: Plane,
    image_release: Release,
}

/// Debian's libheif with its decoder plugins, loaded while this process may still open files.
pub(crate) struct Libheif {
    api: Api,
    _library: Library,
}

struct Owned<'a> {
    pointer: *mut c_void,
    release: &'a Release,
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: pointer came from the libheif call that release belongs to and is released
            // only here.
            unsafe { (self.release)(self.pointer) };
        }
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, Failure> {
    // SAFETY: the caller passes the C prototype of name as T.
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| {
            tracing::warn!("libheif lacks a function: {error}");
            Failure::MissingDecoder
        })
}

fn checked(error: HeifError) -> Result<(), Failure> {
    match error.code {
        OK => Ok(()),
        PLUGIN_LOADING_ERROR => Err(Failure::MissingDecoder),
        _ => Err(Failure::Damaged),
    }
}

impl Libheif {
    pub(crate) fn load() -> Result<Self, Failure> {
        // SAFETY: libheif's initialisers have no preconditions on the loading process.
        let library = unsafe { Library::new(LIBRARY) }.map_err(|error| {
            tracing::warn!("cannot load libheif: {error}");
            Failure::MissingDecoder
        })?;
        // SAFETY: each type is the C prototype of the libheif 1.23 function it is loaded as.
        let (init, api) = unsafe {
            (
                symbol::<Init>(&library, b"heif_init\0")?,
                Api {
                    deinit: symbol(&library, b"heif_deinit\0")?,
                    have_decoder: symbol(&library, b"heif_have_decoder_for_format\0")?,
                    context_alloc: symbol(&library, b"heif_context_alloc\0")?,
                    context_free: symbol(&library, b"heif_context_free\0")?,
                    read_from_memory: symbol(
                        &library,
                        b"heif_context_read_from_memory_without_copy\0",
                    )?,
                    primary_handle: symbol(&library, b"heif_context_get_primary_image_handle\0")?,
                    handle_width: symbol(&library, b"heif_image_handle_get_width\0")?,
                    handle_height: symbol(&library, b"heif_image_handle_get_height\0")?,
                    handle_release: symbol(&library, b"heif_image_handle_release\0")?,
                    options_alloc: symbol(&library, b"heif_decoding_options_alloc\0")?,
                    options_free: symbol(&library, b"heif_decoding_options_free\0")?,
                    decode_image: symbol(&library, b"heif_decode_image\0")?,
                    image_width: symbol(&library, b"heif_image_get_width\0")?,
                    image_height: symbol(&library, b"heif_image_get_height\0")?,
                    plane: symbol(&library, b"heif_image_get_plane_readonly2\0")?,
                    image_release: symbol(&library, b"heif_image_release\0")?,
                },
            )
        };
        // SAFETY: a null parameter asks for the default plugin folder; each init is matched by
        // the deinit in Drop.
        checked(unsafe { init(ptr::null()) })?;
        Ok(Self {
            api,
            _library: library,
        })
    }

    pub(crate) fn decode(&self, bytes: &[u8], kind: Kind) -> Result<RgbaImage, Failure> {
        let api = &self.api;
        let context = Owned {
            // SAFETY: heif_context_alloc has no preconditions.
            pointer: unsafe { (api.context_alloc)() },
            release: &api.context_free,
        };
        if context.pointer.is_null() {
            return Err(Failure::Damaged);
        }
        // SAFETY: bytes outlives context, which reads them without copying, and null asks for
        // the default reading options.
        checked(unsafe {
            (api.read_from_memory)(
                context.pointer,
                bytes.as_ptr().cast(),
                bytes.len(),
                ptr::null(),
            )
        })?;
        let mut handle = Owned {
            pointer: ptr::null_mut(),
            release: &api.handle_release,
        };
        // SAFETY: context is a live context and handle.pointer receives a handle we release.
        checked(unsafe { (api.primary_handle)(context.pointer, &mut handle.pointer) })?;
        // SAFETY: handle is a live image handle.
        let side = |get: HandleSide| u32::try_from(unsafe { get(handle.pointer) });
        let (width, height) = side(api.handle_width)
            .and_then(|width| Ok((width, side(api.handle_height)?)))
            .map_err(|_| Failure::Damaged)?;
        allowed(width, height)?;
        // SAFETY: heif_decoding_options_alloc has no preconditions.
        let options = unsafe { (api.options_alloc)() };
        if options.is_null() {
            return Err(Failure::Damaged);
        }
        // SAFETY: options is libheif's own allocation of at least the version it states, and
        // convert_hdr_to_8bit exists from version 2 on.
        unsafe {
            if (*options).version >= HDR_TO_8BIT_VERSION {
                (*options).convert_hdr_to_8bit = 1;
            }
        }
        let mut image = Owned {
            pointer: ptr::null_mut(),
            release: &api.image_release,
        };
        // SAFETY: handle and options are live, and image.pointer receives an image we release.
        let decoded = unsafe {
            (api.decode_image)(
                handle.pointer,
                &mut image.pointer,
                COLORSPACE_RGB,
                CHROMA_INTERLEAVED_RGBA,
                options,
            )
        };
        // SAFETY: options came from heif_decoding_options_alloc and is not used again.
        unsafe { (api.options_free)(options) };
        if decoded.code != OK || image.pointer.is_null() {
            let format = match kind {
                Kind::Avif => COMPRESSION_AV1,
                _ => COMPRESSION_HEVC,
            };
            // SAFETY: heif_have_decoder_for_format has no preconditions.
            if decoded.code == PLUGIN_LOADING_ERROR || unsafe { (api.have_decoder)(format) } == 0 {
                return Err(Failure::MissingDecoder);
            }
            return Err(Failure::Damaged);
        }
        self.pixels(&image)
    }

    fn pixels(&self, image: &Owned<'_>) -> Result<RgbaImage, Failure> {
        let api = &self.api;
        // SAFETY: image is a live decoded image.
        let side =
            |get: ImageSide| u32::try_from(unsafe { get(image.pointer, CHANNEL_INTERLEAVED) });
        let (width, height) = side(api.image_width)
            .and_then(|width| Ok((width, side(api.image_height)?)))
            .map_err(|_| Failure::Damaged)?;
        allowed(width, height)?;
        let mut stride = 0usize;
        // SAFETY: image is live and stride receives the plane's row length in bytes.
        let plane = unsafe { (api.plane)(image.pointer, CHANNEL_INTERLEAVED, &mut stride) };
        let row = width as usize * 4;
        if plane.is_null() || stride < row {
            return Err(Failure::Damaged);
        }
        // SAFETY: libheif's interleaved plane holds height rows of stride bytes and lives as long
        // as image, which outlives this slice.
        let data = unsafe { std::slice::from_raw_parts(plane, stride * height as usize) };
        let rgba = data
            .chunks_exact(stride)
            .flat_map(|line| &line[..row])
            .copied()
            .collect();
        RgbaImage::from_raw(width, height, rgba).ok_or(Failure::Damaged)
    }
}

impl Drop for Libheif {
    fn drop(&mut self) {
        // SAFETY: matches the heif_init in load; nothing from libheif outlives self.
        unsafe { (self.api.deinit)() };
    }
}
