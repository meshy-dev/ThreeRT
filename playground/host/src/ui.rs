//! The PocketJS side: one UiProgram (QuickJS guest + core + Runtime) laid out and drawn by pocket-web-render
//! (wgpu 30) on the host's device. Mirrors pocket-ui-wgpu::UiOverlay; the UI is recorded straight into the host's
//! command encoder (`WgpuRenderer::encode`), so a frame never waits on the GPU.

use crate::output::{Gpu, ProgramOutput, ProgramTarget};
use anyhow::{Context, Result, anyhow};
use pocket_mod::{Guest, qjs::Function};
use pocket_ui_surface::{ProgramConfig, UiProgram, UiSurface, embedded_profile};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Panel background behind everything (UE5 "Background").
const CLEAR: wgpu::Color = wgpu::Color { r: 0.0080, g: 0.0080, b: 0.0080, a: 1.0 };

pub struct PocketUi {
    program: UiProgram,
    pub surface: UiSurface,
    guest: Rc<Guest>,
    _output: ProgramOutput,
    /// Compositor surfaces published to the guest as `ui.__surfaces.<name>` (<CompositorSurface package=name>).
    pub surfaces: BTreeMap<String, u32>,
    /// Nodes the UI registered with `overlay.control(name, node)`.
    controls: BTreeMap<String, i32>,
    pub size: (u32, u32),
    pub scale: f32,
    renderer: pocket_web_render::WgpuRenderer,
    target: Option<Arc<ProgramTarget>>,
}

impl PocketUi {
    pub fn new(gpu: &Gpu, bundle: &str, pak: &[u8], size: (u32, u32), scale: f32, surfaces: &[String]) -> Result<Self> {
        let logical = [(size.0 as f32 / scale).ceil() as u32, (size.1 as f32 / scale).ceil() as u32];
        let (profile, profile_hash) = embedded_profile("threert-playground", logical, 2, 60);
        let mut program = UiProgram::new(ProgramConfig {
            profile,
            profile_hash,
            tick_hz: 60,
            host_abi: pocket_spec::BINDING_GENERATION,
            logical,
            heap_limit: 128 * 1024 * 1024,
            frame_buffer_budget: 256 * 1024 * 1024,
        })?;
        let surface = program.surface();
        surface.set_svc_allowlist(["pocket.overlay"]);
        let mut handles = BTreeMap::new();
        for name in surfaces {
            handles.insert(name.clone(), surface.register_compositor_surface(name).with_context(|| format!("surface {name}"))? as u32);
        }
        let guest = Rc::new(Guest::new()?);
        let output = ProgramOutput::with_options(gpu, (logical[0], logical[1]), 1., FORMAT, CLEAR, false)?;
        output.register(&mut program)?;
        program.boot(guest.clone(), "threert-playground", bundle, pak)?;
        output.configure(size, scale)?;
        program.runtime_mut().resize_embedded_gpu([size.0 as f32 / scale, size.1 as f32 / scale], [size.0, size.1]).map_err(|e| anyhow!("viewport: {e:?}"))?;
        let renderer = pocket_web_render::WgpuRenderer::new(&gpu.device, &gpu.queue, FORMAT).map_err(|e| anyhow!("ui renderer: {e:?}"))?;
        let ui = Self { program, surface, guest, _output: output, surfaces: handles, controls: BTreeMap::new(), size, scale, renderer, target: None };
        ui.resize_guest()?;
        Ok(ui)
    }

    fn resize_guest(&self) -> Result<()> {
        let (w, h) = (self.size.0 as f32 / self.scale, self.size.1 as f32 / self.scale);
        self.guest.with(|ctx| -> Result<()> {
            let hook: Option<Function> = ctx.globals().get("__pocketResizeViewport")?;
            if let Some(hook) = hook {
                hook.call::<_, ()>((w, h)).map_err(|e| anyhow!("resize: {e}"))?;
            }
            Ok(())
        })
    }

    pub fn resize(&mut self, size: (u32, u32), scale: f32) -> Result<()> {
        if (size, scale) == (self.size, self.scale) || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.pointer(json!({"kind":"cancel"}));
        self._output.configure(size, scale)?;
        self.program.runtime_mut().resize_embedded_gpu([size.0 as f32 / scale, size.1 as f32 / scale], [size.0, size.1]).map_err(|e| anyhow!("resize: {e:?}"))?;
        self.size = size;
        self.scale = scale;
        self.resize_guest()
    }

    pub fn pointer(&self, event: Value) {
        self.surface.svc_push(json!({"type":"pointer","event":event}).to_string());
    }

    /// Application message for the UI's `connectOverlay` receiver.
    pub fn push(&self, value: &Value) {
        self.surface.svc_push(json!({"type":"state","value":value}).to_string());
    }

    /// One guest turn + core tick. Returns the commands the UI sent.
    pub fn advance(&mut self) -> Result<Vec<Value>> {
        self.program.advance()?;
        let mut commands = vec![];
        for line in self.surface.svc_drain() {
            let v: Value = serde_json::from_str(&line)?;
            if v["type"] == "pocket.overlay.control" {
                if let (Some(name), Some(node)) = (v["name"].as_str(), v["node"].as_i64()) {
                    if v["remove"] == true {
                        self.controls.remove(name);
                    } else {
                        self.controls.insert(name.to_owned(), node as i32);
                    }
                }
            } else {
                commands.push(v);
            }
        }
        Ok(commands)
    }

    /// Logical rect [x, y, w, h] of a compositor surface as painted this frame.
    pub fn surface_rect(&self, name: &str) -> Option<[f32; 4]> {
        let h = *self.surfaces.get(name)?;
        self.surface.with_ui(|ui| ui.compositor_surface_frames().into_iter().find(|f| f.handle == h).map(|f| f.full))
    }

    /// Is the topmost painted node under (x, y) (logical) the control registered as `name`?
    pub fn over(&self, name: &str, x: f32, y: f32) -> bool {
        let Some(&node) = self.controls.get(name) else { return false };
        self.surface.with_ui(|ui| ui.hit_test(x, y)) == node
    }

    /// Logical bounds [x, y, w, h] of a named control as painted (cold diagnostic query, for automation).
    pub fn control_bounds(&self, name: &str) -> Option<[f32; 4]> {
        let &node = self.controls.get(name)?;
        self.surface.with_ui(|ui| {
            ui.debug_inspect(node);
            ui.draw();
            let (xy, wh) = (ui.debug_rect_xy(), ui.debug_rect_wh());
            ui.debug_inspect(0);
            (wh != -1).then(|| [(xy as i16) as f32, ((xy >> 16) as i16) as f32, (wh & 0xffff) as f32, ((wh >> 16) & 0xffff) as f32])
        })
    }

    /// Binds a texture to a compositor surface for the next encode.
    pub fn bind_surface(&mut self, name: &str, view: &wgpu::TextureView, logical: (u32, u32)) {
        if let Some(&h) = self.surfaces.get(name) {
            self.renderer.set_surface(h, view, logical);
        }
    }

    /// Records the UI (with its compositor surfaces) into `e`; returns the full-window UI target.
    pub fn encode(&mut self, device: &wgpu::Device, e: &mut wgpu::CommandEncoder) -> Result<Arc<ProgramTarget>> {
        let size = self.size;
        if self.target.as_ref().map(|t| t.size) != Some(size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ui target"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.target = Some(Arc::new(ProgramTarget { texture, view, size }));
        }
        let tgt = self.target.clone().unwrap();
        let (scale, r) = (self.scale, &mut self.renderer);
        self.surface.with_ui(|ui| -> Result<()> {
            let words = ui.draw().words.clone();
            r.encode(ui, &words, e, &tgt.view, tgt.size, scale, wgpu::LoadOp::Clear(CLEAR)).map_err(|e| anyhow!("ui encode: {e:?}"))?.retain_in_encoder();
            Ok(())
        })?;
        Ok(tgt)
    }
}
