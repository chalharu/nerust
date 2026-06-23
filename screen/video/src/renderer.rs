use crate::{FrameBuffer, SurfaceSize};

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
}
