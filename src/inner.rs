#![cfg(any(feature = "gtk", feature = "tao"))]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use clap::{Arg, Command};
use log::LevelFilter;
use nerust_factory_nes::NesFactory;
use nerust_gui_runtime::rom::load_rom_path;
use nerust_gui_shell::{
    context::FrontendContext,
    factory::CoreFactory,
    load::{MediaObject, RomLoadTarget, RomLoader, RomLoaderError, SystemLoadOptions},
};
use nerust_run_options::RunOptions;
use nerust_screen_video::GpuFactory;
use simple_logger::SimpleLogger;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(all(feature = "wgpu", not(feature = "opengl")))]
    return Box::new(nerust_backend_wgpu::WgpuFactory);
    #[cfg(all(feature = "opengl", not(feature = "wgpu")))]
    return Box::new(nerust_backend_opengl::GlFactory);
    #[cfg(not(any(feature = "wgpu", feature = "opengl")))]
    compile_error!("No backend selected. Enable feature 'wgpu' or 'opengl'.");
    #[cfg(all(feature = "wgpu", feature = "opengl"))]
    compile_error!("Multiple backends selected. Enable only one of 'wgpu' or 'opengl'.");
}

fn parse_cli_args() -> RunOptions {
    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(Arg::new("filename").help("Rom file name"))
        .arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        );

    let matches = app.get_matches();
    RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
        mmc3_irq_variant: matches.get_one::<String>("mmc3-irq-variant").cloned(),
    }
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
        let resolved = self
            .factory
            .resolve_load_request(target.settings_snapshot(), options)
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

    let options = parse_cli_args();
    let gpu_factory = create_factory();
    let core_factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);

    let pending_options = options.mmc3_irq_variant.as_deref().map(|variant| {
        let options_bytes = match variant {
            "sharp" => nerust_factory_nes::MMC3_OPTION_SHARP.to_vec(),
            "nec" => nerust_factory_nes::MMC3_OPTION_NEC.to_vec(),
            _ => Vec::new(),
        };
        SystemLoadOptions { options_bytes }
    });

    let rom_loader = Box::new(LiveRomLoader {
        factory: Arc::clone(&core_factory),
        pending_options,
    });

    let ctx = FrontendContext {
        gpu_factory: Rc::from(gpu_factory),
        core_factory,
        rom_loader,
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

    use nerust_factory_nes::NesFactory;
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
