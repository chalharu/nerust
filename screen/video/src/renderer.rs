use crate::{FrameBuffer, SurfaceSize, VideoRenderProfile};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Wraps a static or formatted message as an `std::error::Error`.
///
/// Used when no meaningful typed error is available (e.g. `glXChooseFBConfig`
/// returned null without setting an error).
#[derive(Debug)]
pub struct OpaqueError(pub String);

impl std::fmt::Display for OpaqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OpaqueError {}

/// Renderer 関連のエラー。
///
/// 単一構造体で、`context` に発生箇所の説明（"display init" / "surface recreate" 等）、
/// `source` に元のエラー型を保持する。OCP を満たし、新 backend は既存型を変えずに
/// 任意の context 文字列を使える。
///
/// Frontend は `Display` で `"display init: (source message)"` と表示でき、
/// `source()` → `downcast_ref()` で具体型を取り出せる。
#[derive(Debug)]
pub struct RendererError {
    context: &'static str,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl RendererError {
    pub fn new(context: &'static str, source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self { context, source }
    }
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.source)
    }
}

impl std::error::Error for RendererError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

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
    ) -> Result<(), RendererError> {
        Err(RendererError::new(
            "surface recreate",
            Box::new(OpaqueError("not supported by this backend".to_string())),
        ))
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
    ) -> Result<Box<dyn Renderer>, RendererError>;
}
