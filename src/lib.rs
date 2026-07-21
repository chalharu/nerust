use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use clap::Command;
use log::LevelFilter;
use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::{
        CoreFactory,
        load::{DynSystemLoadOptions, MediaObject},
    },
};
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_shell::{
    context::FrontendContext,
    load::{RomLoadTarget, RomLoader, RomLoaderError},
    settings::factory::settings_view,
};
use nerust_nes_factory::NesFactory;
use nerust_render_traits::renderer::GpuFactory;
use nerust_run_options::RunOptions;
use simple_logger::SimpleLogger;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(all(feature = "wgpu", not(any(feature = "opengl", feature = "softbuffer"))))]
    return Box::new(nerust_render_wgpu::WgpuFactory);
    #[cfg(all(feature = "opengl", not(any(feature = "wgpu", feature = "softbuffer"))))]
    return Box::new(nerust_render_gl::GlFactory);
    #[cfg(all(feature = "softbuffer", not(any(feature = "wgpu", feature = "opengl"))))]
    return Box::new(nerust_render_softbuffer::SoftbufferFactory);
    #[cfg(not(any(feature = "wgpu", feature = "opengl", feature = "softbuffer")))]
    compile_error!("No backend selected. Enable feature 'wgpu', 'opengl' or 'softbuffer'.");
    #[cfg(any(
        all(feature = "wgpu", feature = "opengl"),
        all(feature = "wgpu", feature = "softbuffer"),
        all(feature = "opengl", feature = "softbuffer"),
    ))]
    compile_error!(
        "Multiple backends selected. Enable only one of 'wgpu', 'opengl' or 'softbuffer'."
    );
}

fn create_audio_registry() -> AudioBackendRegistry {
    #[cfg_attr(not(any(feature = "gtk", feature = "tao")), allow(unused_mut))]
    let mut reg = AudioBackendRegistry::new();
    #[cfg(any(feature = "gtk", feature = "tao"))]
    reg.register(0, Box::new(nerust_sound_cpal::CpalFactory));
    #[cfg(any(feature = "gtk", feature = "tao"))]
    reg.register(1, Box::new(nerust_sound_cubeb::CubebFactory));
    reg
}

fn parse_cli_args(
    factories: &[Arc<dyn CoreFactory>],
) -> (RunOptions, Vec<Box<dyn DynSystemLoadOptions>>) {
    let defaults: Vec<_> = factories.iter().map(|f| f.default_load_options()).collect();

    let mut app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(clap::Arg::new("filename").help("Rom file name"));
    for opt in &defaults {
        app = opt.augment_args(app);
    }

    let matches = app.get_matches();
    let options = RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
    };
    let parsed = defaults
        .iter()
        .map(|opt| opt.arg_matches(&matches))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (options, parsed)
}

struct LiveRomLoader {
    factory: Arc<dyn CoreFactory>,
    pending_options: Option<Box<dyn DynSystemLoadOptions>>,
}

impl RomLoader for LiveRomLoader {
    fn load_rom(
        &mut self,
        path: &Path,
        target: &mut dyn RomLoadTarget,
    ) -> Result<(), RomLoaderError> {
        let loaded = load_rom_path(path).map_err(|e| RomLoaderError::Io(e.to_string()))?;
        let (rom_path, data) = loaded.into_parts();
        let media = MediaObject::new(Some(rom_path), data);
        let options = self
            .pending_options
            .take()
            .unwrap_or_else(|| target.default_load_options());
        let system_id = self.factory.system_id();
        let view = settings_view(target.settings_snapshot(), &system_id);
        let resolved = self
            .factory
            .resolve_load_request(&view, options)
            .map_err(|e| RomLoaderError::Resolve(e.to_string()))?;
        target.load_resolved(media, resolved)?;
        target.resume();
        Ok(())
    }
}

pub fn run() {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .init()
        .unwrap();

    let gpu_factory = create_factory();
    let core_factories: Vec<Arc<dyn CoreFactory>> = vec![Arc::new(NesFactory)];
    let audio_registry = Arc::new(create_audio_registry());

    let (options, core_options) = parse_cli_args(&core_factories);

    let rom_loaders = core_factories
        .iter()
        .zip(core_options)
        .map(|(f, o)| {
            Box::new(LiveRomLoader {
                factory: f.clone(),
                pending_options: Some(o),
            }) as Box<dyn RomLoader>
        })
        .collect::<Vec<_>>();

    // TODO: 本当は複数の RomLoader をまとめるような構造体を作るべきだが、今は NES しかないので最初の一つだけ使う
    let rom_loader = rom_loaders
        .into_iter()
        .next()
        .expect("No RomLoader available");

    let ctx = FrontendContext {
        gpu_factory: Rc::from(gpu_factory),
        // TODO: 本当は複数の CoreFactory をまとめるような構造体を作るべきだが、今は NES しかないので最初の一つだけ使う
        core_factory: core_factories
            .into_iter()
            .next()
            .expect("No CoreFactory available"),
        rom_loader,
        audio_registry,
    };

    #[cfg(all(feature = "gtk", not(clippy)))]
    nerust_gtk::run(ctx, options);
    #[cfg(all(feature = "tao", not(clippy)))]
    nerust_tao::run(ctx, options);
    #[cfg(not(any(feature = "gtk", feature = "tao", clippy)))]
    compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");
    #[cfg(clippy)]
    {
        let _ = ctx;
        let _ = options;
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Arc};

    use nerust_core_traits::factory::{
        CoreFactory,
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    };
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::{
        load::{RomLoadTarget, RomLoader, RomLoaderError},
        settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        },
    };
    use nerust_nes_factory::NesFactory;

    use super::LiveRomLoader;

    struct LoadRecorder {
        resolved: Option<Box<dyn DynSystemLoadOptions>>,
        resumed: bool,
        snapshot: SettingsSnapshot,
    }

    impl RomLoadTarget for LoadRecorder {
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            unreachable!()
        }
        fn settings_snapshot(&self) -> &SettingsSnapshot {
            &self.snapshot
        }
        fn load_resolved(
            &mut self,
            _media: MediaObject,
            resolved: ResolvedLoadRequest,
        ) -> Result<(), RomLoaderError> {
            self.resolved = Some(resolved.options);
            Ok(())
        }
        fn resume(&mut self) {
            self.resumed = true;
        }
    }

    #[test]
    fn live_rom_loader_uses_pending_options_when_set() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let pending = Some(factory.default_load_options());
        let mut loader = LiveRomLoader {
            factory: factory.clone(),
            pending_options: pending,
        };
        fn make_snapshot() -> SettingsSnapshot {
            SettingsSnapshot {
                shared: default_shared_settings(),
                local: default_local_settings(),
                app_state: default_app_state(),
            }
        }

        let mut target = LoadRecorder {
            resolved: None,
            resumed: false,
            snapshot: make_snapshot(),
        };

        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target);
        assert!(result.is_ok());
        assert!(target.resumed);
        assert!(target.resolved.is_some(), "expected non-empty core options");
    }

    #[test]
    fn live_rom_loader_pending_options_consumed_once() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let pending = Some(factory.default_load_options());
        let mut loader = LiveRomLoader {
            factory: factory.clone(),
            pending_options: pending,
        };
        fn make_snap() -> SettingsSnapshot {
            SettingsSnapshot {
                shared: default_shared_settings(),
                local: default_local_settings(),
                app_state: default_app_state(),
            }
        }

        let mut target = LoadRecorder {
            resolved: None,
            resumed: false,
            snapshot: make_snap(),
        };

        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target);
        assert!(result.is_ok());
        assert!(target.resumed, "expected resume on first load");

        let mut target2 = LoadRecorder {
            resolved: None,
            resumed: false,
            snapshot: make_snap(),
        };
        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target2);
        assert!(result.is_ok());
        assert!(target2.resumed, "expected resume on second load");
    }
}
