use std::{collections::HashMap, path::PathBuf, rc::Rc, sync::Arc};

use clap::Command;
use log::LevelFilter;
use nerust_core_traits::{
    audio::AudioBackendRegistry,
    factory::{CoreFactory, load::DynSystemLoadOptions},
    identity::SystemId,
};
use nerust_gui_shell::{context::FrontendContext, registry::SystemRegistry};
use nerust_render_traits::renderer::GpuFactory;
use nerust_run_options::RunOptions;
use simple_logger::SimpleLogger;

type LoadOptionsBySystem = HashMap<Box<dyn SystemId>, Box<dyn DynSystemLoadOptions>>;
type CliParseResult = Result<(RunOptions, LoadOptionsBySystem), clap::Error>;

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

fn parse_cli_args(factories: &[Arc<dyn CoreFactory>]) -> CliParseResult {
    parse_cli_args_from(factories, std::env::args())
}

fn parse_cli_args_from(
    factories: &[Arc<dyn CoreFactory>],
    args: impl IntoIterator<Item = String>,
) -> CliParseResult {
    let defaults: Vec<_> = factories.iter().map(|f| f.load_options_schema()).collect();
    validate_unique_cli_arguments(factories, &defaults)?;

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
    let parsed = factories
        .iter()
        .zip(&defaults)
        .map(|(factory, schema)| {
            schema
                .arg_matches(&matches)
                .map(|options| (factory.system_id(), options))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok((options, parsed))
}

fn validate_unique_cli_arguments(
    factories: &[Arc<dyn CoreFactory>],
    schemas: &[Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema>],
) -> Result<(), clap::Error> {
    let mut owners: HashMap<String, Box<dyn SystemId>> = HashMap::new();
    for (factory, schema) in factories.iter().zip(schemas) {
        let system_id = factory.system_id();
        let command = schema.augment_args(Command::new("system-options"));
        for argument in command.get_arguments() {
            let argument_id = argument.get_id().as_str().to_string();
            if let Some(previous) = owners.insert(argument_id.clone(), system_id.clone()) {
                return Err(clap::Error::raw(
                    clap::error::ErrorKind::ArgumentConflict,
                    format!(
                        "CLI argument '{argument_id}' is declared by both {previous} and {system_id}"
                    ),
                ));
            }
        }
    }
    Ok(())
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

    let rom_loader = registry
        .create_loader(core_options)
        .expect("CLI options must belong to registered systems");

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
    use std::{collections::HashMap, sync::Arc};

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
        let system_id = factory.system_id();
        let pending = factory.default_load_options();
        let shared = default_shared_settings(std::slice::from_ref(&factory));
        let registry = Arc::new(SystemRegistry::new(vec![factory]));
        let mut loader = registry
            .create_loader(HashMap::from([(system_id, pending)]))
            .unwrap();

        let mut target = LoadRecorder {
            resolved: None,
            resumed: false,
            snapshot: SettingsSnapshot {
                shared,
                local: default_local_settings(),
                app_state: default_app_state(),
            },
        };

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
        assert!(
            parsed.contains_key(&factories[0].system_id()),
            "factory options should be keyed by system ID"
        );
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
