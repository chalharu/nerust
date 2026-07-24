use std::{path::PathBuf, rc::Rc, sync::Arc};

use clap::Command;
use log::LevelFilter;
use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::{CoreFactory, load::DynSystemLoadOptions},
};
use nerust_gui_shell::{context::FrontendContext, registry::SystemRegistry};
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
) -> Result<(RunOptions, Vec<Box<dyn DynSystemLoadOptions>>), clap::Error> {
    parse_cli_args_from(factories, std::env::args())
}

fn parse_cli_args_from(
    factories: &[Arc<dyn CoreFactory>],
    args: impl IntoIterator<Item = String>,
) -> Result<(RunOptions, Vec<Box<dyn DynSystemLoadOptions>>), clap::Error> {
    let defaults: Vec<_> = factories.iter().map(|f| f.load_options_schema()).collect();

    let mut app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(clap::Arg::new("filename").help("Rom file name"));
    for opt in &defaults {
        app = opt.augment_args(app);
    }

    let matches = app.try_get_matches_from(args)?;
    let options = RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
    };
    let parsed = defaults
        .iter()
        .map(|opt| opt.arg_matches(&matches))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((options, parsed))
}

pub fn run() {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .init()
        .unwrap();

    let gpu_factory = create_factory();
    let factories: Vec<Arc<dyn CoreFactory>> = vec![
        #[cfg(feature = "nes")]
        Arc::new(nerust_nes_factory::NesFactory),
    ];
    let registry = Arc::new(SystemRegistry::new(factories));
    let audio_registry = Arc::new(create_audio_registry());

    let (options, core_options) = parse_cli_args(registry.all()).unwrap_or_else(|e| e.exit());

    let rom_loader = registry.create_loader(core_options);

    let ctx = FrontendContext {
        gpu_factory: Rc::from(gpu_factory),
        registry,
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
    use std::sync::Arc;

    use nerust_core_traits::{
        CoreOptions,
        factory::{
            CoreFactory,
            load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
        },
    };
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::{
        load::{RomLoadTarget, RomLoaderError},
        registry::SystemRegistry,
        settings::defaults::seed::{
            default_app_state, default_local_settings, default_shared_settings,
        },
    };
    #[cfg(feature = "nes")]
    use nerust_nes_factory::NesFactory;

    struct LoadRecorder {
        resolved: Option<Box<dyn CoreOptions>>,
        resumed: bool,
        snapshot: SettingsSnapshot,
    }

    impl RomLoadTarget for LoadRecorder {
        fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
            None
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
    fn registry_rom_loader_uses_pending_options() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let pending = factory.default_load_options();
        let registry = SystemRegistry::new(vec![factory]);
        let mut loader = registry.create_loader(vec![pending]);

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

        // Write a minimal valid NES ROM to a temp file
        let rom_path = std::env::temp_dir().join("nerust_test_rom.nes");
        let nes_bytes = vec![0x4E, 0x45, 0x53, 0x1A, 1u8, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        std::fs::write(&rom_path, &nes_bytes).expect("write rom");

        let result = loader.load_rom(&rom_path, &mut target);
        let _ = std::fs::remove_file(&rom_path);
        assert!(result.is_ok());
        assert!(target.resumed);
        assert!(target.resolved.is_some(), "expected non-empty core options");
    }

    #[test]
    fn parse_cli_args_from_returns_default_options_with_no_system_args() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let factories = [factory];

        let result = super::parse_cli_args_from(&factories, ["nerust".into()]);

        let (_options, parsed) = result.expect("parse should succeed with no args");
        assert_eq!(parsed.len(), 1, "one factory should produce one option set");
    }

    #[test]
    fn parse_cli_args_from_accepts_mmc3_irq_variant_flag() {
        let factory: Arc<dyn CoreFactory> = Arc::new(NesFactory);
        let factories = [factory];

        let result = super::parse_cli_args_from(
            &factories,
            ["nerust".into(), "--mmc3-irq-variant".into(), "sharp".into()],
        );

        assert!(result.is_ok(), "valid flag should parse without error");
    }
}
