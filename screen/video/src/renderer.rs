use crate::{FrameBuffer, SurfaceSize, VideoRenderProfile};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Outcome reported by [`Renderer::render`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    /// A frame was successfully presented.
    Presented,
    /// The frame was skipped (surface not ready, resize in flight, etc.).
    Skipped,
    /// A render error occurred; the shell may log or surface this as needed.
    Error,
}

/// 画面描画の抽象化。
///
/// 実装はフレームバッファを受け取り、画面に出力する。
/// `PixelFormat::PaletteIndex` の場合、実装は自動的に palette texture
/// を同期し、GPU 側で RGB デコードを行う。
pub trait Renderer: std::fmt::Debug {
    /// フレームバッファを表示する。
    /// FrameBuffer は自身のサイズ・PixelFormat を知っている。
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;

    /// サーフェイスサイズを変更する。
    fn reconfigure(&mut self, size: SurfaceSize);

    /// ネイティブサーフェイスを再作成する（例: wgpu surface loss on Android）。
    /// デフォルト実装は unsupported。
    fn recreate_surface(
        &mut self,
        _window_handle: RawWindowHandle,
        _display_handle: RawDisplayHandle,
        _size: SurfaceSize,
    ) -> Result<(), String> {
        Err("surface recreation not supported by this backend".to_string())
    }
}

/// Renderer 構築に必要な共通パラメータ。
pub struct RendererConfig {
    pub initial_size: SurfaceSize,
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
}

/// Renderer のファクトリ。
///
/// 各 backend は ZST として実装し、`RendererConfig` から `Box<dyn Renderer>` を生成する。
/// Backend 固有パラメータ（`DeviceLimitProfile` 等）は impl 内部で吸収する。
pub trait RendererFactory {
    /// Renderer を構築する。
    fn create_renderer(
        &self,
        config: &RendererConfig,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn Renderer>, String>;
}
