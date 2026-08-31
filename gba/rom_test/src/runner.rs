use std::path::Path;

use crate::manifest::RomManifest;

pub fn run_manifest(_rom_root: &Path, _manifest: &RomManifest) -> Vec<crate::report::CaseResult> {
    Vec::new()
}
