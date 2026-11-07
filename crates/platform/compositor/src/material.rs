use std::collections::HashMap;
use std::path::PathBuf;

use bande_graphics::{Material, Scheme, Style, limit_backdrop};
use bande_render::{VulkanRenderer, VulkanTexture};
use bande_wallpaper::{Choice, Fit, Variant, material};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::ImportMem;

use crate::state::{Compositor, State};

const MATERIAL_WIDTH: i32 = 480;

#[derive(Debug, Clone, PartialEq)]
struct Wanted {
    path: PathBuf,
    fit: Fit,
    width: u32,
    height: u32,
    scheme: Scheme,
}

pub struct MaterialRequest {
    output: String,
    generation: u64,
    wanted: Wanted,
}

pub struct MaterialPicture {
    output: String,
    generation: u64,
    width: i32,
    height: i32,
    rgba: Vec<u8>,
}

pub fn render_material(request: &MaterialRequest) -> Option<MaterialPicture> {
    let wanted = &request.wanted;
    let scheme = wanted.scheme;
    let picture = material(
        &wanted.path,
        wanted.fit,
        wanted.width,
        wanted.height,
        |color| limit_backdrop(color, scheme),
    )
    .inspect_err(|error| {
        tracing::warn!(
            "cannot make the wallpaper material from {}: {error}",
            wanted.path.display()
        );
    })
    .ok()?;
    Some(MaterialPicture {
        output: request.output.clone(),
        generation: request.generation,
        width: picture.width as i32,
        height: picture.height as i32,
        rgba: picture.rgba,
    })
}

#[derive(Clone)]
pub(crate) struct MaterialTexture {
    pub(crate) texture: VulkanTexture,
    pub(crate) commit: u64,
}

#[derive(Default)]
pub(crate) struct Materials {
    style: Style,
    increased_contrast: bool,
    choice: Option<Choice>,
    worker: Option<Box<dyn Fn(MaterialRequest)>>,
    generation: u64,
    wanted: HashMap<String, Wanted>,
    pending: Vec<MaterialPicture>,
    textures: HashMap<String, MaterialTexture>,
    uploads: u64,
}

impl State {
    pub(crate) fn material(&self) -> Material {
        if self.materials.increased_contrast || self.assistive.settings.reduce_transparency {
            return Material::Opaque;
        }
        self.materials.style.look().material
    }

    pub(crate) fn refresh_materials(&mut self) {
        if self.material() != Material::Wallpaper {
            self.materials.wanted.clear();
            self.materials.pending.clear();
            self.materials.textures.clear();
            return;
        }
        let Some(choice) = self.materials.choice.clone() else {
            return;
        };
        let scheme = self.decorations.scheme;
        let variant = match scheme {
            Scheme::Light => Variant::Light,
            Scheme::Dark => Variant::Dark,
        };
        let home = std::env::home_dir();
        let data_dirs = bande_xdg::data_dirs();
        let Some(path) = choice.path(variant, home.as_deref(), &data_dirs) else {
            return;
        };
        let names: Vec<(String, i32, i32)> = self
            .screens
            .iter()
            .map(|screen| {
                let size = screen.settings.transform.transform_size(screen.mode_size());
                (screen.name(), size.w, size.h)
            })
            .collect();
        for (output, width, height) in names {
            let wanted = Wanted {
                path: path.clone(),
                fit: choice.fit,
                width: MATERIAL_WIDTH as u32,
                height: (MATERIAL_WIDTH * height / width.max(1)).max(1) as u32,
                scheme,
            };
            if self.materials.wanted.get(&output) == Some(&wanted) {
                continue;
            }
            self.materials.wanted.insert(output.clone(), wanted.clone());
            self.materials.generation += 1;
            if let Some(worker) = &self.materials.worker {
                worker(MaterialRequest {
                    output,
                    generation: self.materials.generation,
                    wanted,
                });
            }
        }
    }

    pub(crate) fn material_texture(&self, output: &str) -> Option<MaterialTexture> {
        self.materials.textures.get(output).cloned()
    }

    pub(crate) fn upload_materials(&mut self, renderer: &mut VulkanRenderer) {
        for picture in std::mem::take(&mut self.materials.pending) {
            let imported = renderer.import_memory(
                &picture.rgba,
                Fourcc::Abgr8888,
                (picture.width, picture.height).into(),
                false,
            );
            match imported {
                Ok(texture) => {
                    self.materials.uploads += 1;
                    self.materials.textures.insert(
                        picture.output,
                        MaterialTexture {
                            texture,
                            commit: self.materials.uploads,
                        },
                    );
                }
                Err(error) => tracing::warn!("cannot upload the wallpaper material: {error}"),
            }
        }
    }
}

impl Compositor {
    pub fn set_material_worker(&mut self, worker: impl Fn(MaterialRequest) + 'static) {
        self.state.materials.worker = Some(Box::new(worker));
        self.state.refresh_materials();
    }

    pub fn set_style(&mut self, style: Style, increased_contrast: bool) {
        self.state.materials.style = style;
        self.state.materials.increased_contrast = increased_contrast;
        self.state.refresh_materials();
        self.state.needs_redraw = true;
    }

    pub fn set_wallpaper_choice(&mut self, choice: Choice) {
        self.state.materials.choice = Some(choice);
        self.state.refresh_materials();
    }

    pub fn finish_material(&mut self, picture: MaterialPicture) {
        let current = self.state.materials.wanted.contains_key(&picture.output)
            && picture.generation <= self.state.materials.generation;
        if current {
            self.state.materials.pending.retain(|pending| {
                pending.output != picture.output || pending.generation > picture.generation
            });
            self.state.materials.pending.push(picture);
            self.state.needs_redraw = true;
        }
    }
}
