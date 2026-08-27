use std::{fmt::Debug, path::PathBuf, sync::Arc};

use clap::{ArgMatches, Args, Command, FromArgMatches};
use downcast_rs::Downcast;
use dyn_eq::DynEq;

use crate::CoreOptions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaLocation {
    NativePath(PathBuf),
    DocumentUri { uri: String, display_name: String },
}

#[derive(Debug, Clone)]
pub struct MediaObject {
    pub bytes: Arc<[u8]>,
    pub location: Option<MediaLocation>,
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
            location: path.clone().map(MediaLocation::NativePath),
            extension,
        }
    }

    pub fn from_document_uri(
        uri: impl Into<String>,
        display_name: impl Into<String>,
        data: Vec<u8>,
    ) -> Self {
        let display_name = display_name.into();
        let extension = PathBuf::from(&display_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        Self {
            bytes: Arc::from(data),
            location: Some(MediaLocation::DocumentUri {
                uri: uri.into(),
                display_name,
            }),
            extension,
        }
    }

    pub fn native_path(&self) -> Option<&std::path::Path> {
        match self.location.as_ref() {
            Some(MediaLocation::NativePath(path)) => Some(path),
            Some(MediaLocation::DocumentUri { .. }) | None => None,
        }
    }
}

#[cfg(test)]
mod media_tests {
    use super::{MediaLocation, MediaObject};

    #[test]
    fn native_media_preserves_path_and_extension() {
        let media = MediaObject::new(Some("game.NES".into()), vec![1]);

        assert_eq!(media.native_path(), Some(std::path::Path::new("game.NES")));
        assert_eq!(media.extension.as_deref(), Some("nes"));
        assert!(matches!(media.location, Some(MediaLocation::NativePath(_))));
    }

    #[test]
    fn document_media_preserves_uri_display_name_and_extension() {
        let media = MediaObject::from_document_uri(
            "content://provider/document/42",
            "Pokemon Crystal.GBC",
            vec![1, 2],
        );

        assert_eq!(media.native_path(), None);
        assert_eq!(media.extension.as_deref(), Some("gbc"));
        assert_eq!(
            media.location,
            Some(MediaLocation::DocumentUri {
                uri: "content://provider/document/42".to_string(),
                display_name: "Pokemon Crystal.GBC".to_string(),
            })
        );
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

#[derive(Clone, Debug)]
pub struct ResolvedLoadRequest {
    pub options: Box<dyn CoreOptions>,
}
