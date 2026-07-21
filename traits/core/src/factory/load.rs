use std::{fmt::Debug, path::PathBuf, sync::Arc};

use clap::{ArgMatches, Args, Command, FromArgMatches};
use downcast_rs::Downcast;
use dyn_eq::DynEq;

use crate::CoreOptions;

#[derive(Debug, Clone)]
pub struct MediaObject {
    pub bytes: Arc<[u8]>,
    pub path: Option<PathBuf>,
    pub extension: Option<String>,
}

impl MediaObject {
    pub fn new(path: Option<PathBuf>, data: Vec<u8>) -> Self {
        let extension = path
            .as_deref()
            .and_then(|p| p.extension())
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        Self {
            bytes: Arc::from(data),
            path,
            extension,
        }
    }
}

pub trait SystemLoadOptions: Args + Debug + Eq + 'static {}

pub trait SystemLoadOptionsSchema: Debug + Eq + 'static {
    type Options: SystemLoadOptions;
}

#[derive(Debug, PartialEq, Eq)]
pub struct SystemLoadOptionsWrapper<T: SystemLoadOptions>(T);

#[derive(Debug, PartialEq, Eq)]
pub struct SystemLoadOptionsSchemaWrapper<T: SystemLoadOptionsSchema>(T);

impl<T: SystemLoadOptionsSchema> SystemLoadOptionsSchemaWrapper<T> {
    pub fn augment_args(&self, cmd: Command) -> Command {
        <T::Options as Args>::augment_args(cmd)
    }

    pub fn arg_matches(
        &self,
        matches: &ArgMatches,
    ) -> Result<SystemLoadOptionsWrapper<T::Options>, clap::Error> {
        <T::Options as FromArgMatches>::from_arg_matches(matches).map(SystemLoadOptionsWrapper)
    }
}

pub trait DynSystemLoadOptions: Debug + DynEq + Downcast {}

pub trait DynSystemLoadOptionsSchema: Debug + DynEq + Downcast {
    fn augment_args(&self, cmd: Command) -> Command;
    fn arg_matches(
        &self,
        matches: &ArgMatches,
    ) -> Result<Box<dyn DynSystemLoadOptions>, clap::Error>;
}

impl<T: SystemLoadOptionsSchema> DynSystemLoadOptionsSchema for SystemLoadOptionsSchemaWrapper<T> {
    fn augment_args(&self, cmd: Command) -> Command {
        SystemLoadOptionsSchemaWrapper::<T>::augment_args(self, cmd)
    }

    fn arg_matches(
        &self,
        matches: &ArgMatches,
    ) -> Result<Box<dyn DynSystemLoadOptions>, clap::Error> {
        SystemLoadOptionsSchemaWrapper::<T>::arg_matches(self, matches).map(|x| Box::new(x) as _)
    }
}

impl<T: SystemLoadOptions> DynSystemLoadOptions for SystemLoadOptionsWrapper<T> {}

downcast_rs::impl_downcast!(DynSystemLoadOptions);
dyn_eq::eq_trait_object!(DynSystemLoadOptions);

downcast_rs::impl_downcast!(DynSystemLoadOptionsSchema);

impl<T: SystemLoadOptions> From<T> for Box<dyn DynSystemLoadOptions> {
    fn from(value: T) -> Self {
        Box::new(SystemLoadOptionsWrapper(value))
    }
}

impl<T: SystemLoadOptionsSchema> From<T> for Box<dyn DynSystemLoadOptionsSchema> {
    fn from(value: T) -> Self {
        Box::new(SystemLoadOptionsSchemaWrapper(value))
    }
}

pub trait DynSystemLoadOptionsExt: Sized {
    fn into_inner<T: SystemLoadOptions>(self) -> Result<T, Self>;
}

impl DynSystemLoadOptionsExt for Box<dyn DynSystemLoadOptions> {
    fn into_inner<T: SystemLoadOptions>(self) -> Result<T, Self> {
        self.downcast::<SystemLoadOptionsWrapper<T>>()
            .map(|wrapper| wrapper.0)
            .map_err(|boxed| boxed as Box<dyn DynSystemLoadOptions>)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLoadRequest {
    pub options: Box<dyn CoreOptions>,
}
