use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use clap::Command;
use log::LevelFilter;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_core_traits::factory::cli::CliProvider;
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_shell::{
    context::FrontendContext,
    factory::CoreFactory,
    load::{MediaObject, RomLoadTarget, RomLoader, RomLoaderError, SystemLoadOptions},
};
use nerust_nes_factory::NesFactory;
use nerust_render_base::GpuFactory;
use nerust_run_options::RunOptions;
use simple_logger::SimpleLogger;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(all(feature = "wgpu", not(feature = "opengl")))]
    return Box::new(nerust_render_wgpu::WgpuFactory);
    #[cfg(all(feature = "opengl", not(feature = "wgpu")))]
    return Box::new(nerust_render_gl::GlFactory);
    #[cfg(not(any(feature = "wgpu", feature = "opengl")))]
    compile_error!("No backend selected. Enable feature 'wgpu' or 'opengl'.");
    #[cfg(all(feature = "wgpu", feature = "opengl"))]
    compile_error!("Multiple backends selected. Enable only one of 'wgpu' or 'opengl'.");
}

fn create_audio_registry() -> AudioBackendRegistry {
    #[cfg_attr(not(any(feature = "gtk", feature = "tao")), allow(unused_mut))]
    let mut reg = AudioBackendRegistry::new();
    #[cfg(any(feature = "gtk", feature = "tao"))]
    reg.register(0, &nerust_sound_cpal::CPAL);
    #[cfg(any(feature = "gtk", feature = "tao"))]
    reg.register(1, &nerust_sound_cubeb::CUBEB);
    #[cfg(all(any(feature = "gtk", feature = "tao"), not(target_os = "android")))]
    reg.register(2, &nerust_sound_openal::OPENAL);
    reg
}

fn parse_cli_args(cli_provider: &dyn CliProvider) -> (RunOptions, Vec<u8>) {
    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(clap::Arg::new("filename").help("Rom file name"));
    let app = cli_provider.extend_command(app);
    let matches = app.get_matches();
    let options = RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
    };
    let core_options = cli_provider.parse_core_options(&matches);
    (options, core_options)
}

struct LiveRomLoader {
    factory: Arc<dyn CoreFactory>,
    pending_options: Option<SystemLoadOptions>,
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
        let view =
            nerust_gui_shell::settings::settings_view(target.settings_snapshot(), &system_id);
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
    let core_factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
    let audio_registry = Arc::new(create_audio_registry());

    let (options, core_options) = parse_cli_args(&NesFactory);
    let pending_options = if core_options.is_empty() {
        None
    } else {
        Some(SystemLoadOptions {
            options_bytes: core_options,
        })
    };

    let rom_loader = Box::new(LiveRomLoader {
        factory: Arc::clone(&core_factory),
        pending_options,
    });

    let ctx = FrontendContext {
        gpu_factory: Rc::from(gpu_factory),
        core_factory,
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
    use std::path::Path;
    use std::sync::Arc;

    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_shell::{
        factory::CoreFactory,
        load::{
            MediaObject, ResolvedLoadRequest, RomLoadTarget, RomLoader, RomLoaderError,
            SystemLoadOptions,
        },
    };
    use nerust_nes_factory::NesFactory;

    use super::LiveRomLoader;

    struct LoadRecorder {
        resolved: Vec<u8>,
        resumed: bool,
        snapshot: SettingsSnapshot,
    }

    impl RomLoadTarget for LoadRecorder {
        fn default_load_options(&self) -> SystemLoadOptions {
            SystemLoadOptions::default()
        }
        fn settings_snapshot(&self) -> &SettingsSnapshot {
            &self.snapshot
        }
        fn load_resolved(
            &mut self,
            _media: MediaObject,
            resolved: ResolvedLoadRequest,
        ) -> Result<(), RomLoaderError> {
            self.resolved = resolved.core_options_bytes;
            Ok(())
        }
        fn resume(&mut self) {
            self.resumed = true;
        }
    }

    #[test]
    fn live_rom_loader_uses_pending_options_when_set() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let pending = Some(SystemLoadOptions {
            options_bytes: b"sharp".to_vec(),
        });
        let mut loader = LiveRomLoader {
            factory,
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
            resolved: Vec::new(),
            resumed: false,
            snapshot: make_snapshot(),
        };

        // load_rom reads from disk; use a path we know exists
        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target);
        assert!(result.is_ok());
        assert!(target.resumed);
        // core_options_bytes should be non-empty (serialized from pending_options)
        assert!(
            !target.resolved.is_empty(),
            "expected non-empty core options"
        );
    }

    #[test]
    fn live_rom_loader_pending_options_consumed_once() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let pending = Some(SystemLoadOptions {
            options_bytes: b"nec".to_vec(),
        });
        let mut loader = LiveRomLoader {
            factory,
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
            resolved: Vec::new(),
            resumed: false,
            snapshot: make_snap(),
        };

        // First call consumes pending_options — resumes
        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target);
        assert!(result.is_ok());
        assert!(target.resumed, "expected resume on first load");

        // Second call uses default options since pending_options was taken — still succeeds
        let mut target2 = LoadRecorder {
            resolved: Vec::new(),
            resumed: false,
            snapshot: make_snap(),
        };
        let result = loader.load_rom(Path::new("Cargo.toml"), &mut target2);
        assert!(result.is_ok());
        assert!(target2.resumed, "expected resume on second load");
    }
}
