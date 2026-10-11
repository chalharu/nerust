use nerust_gui_settings::language::AppLanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiText {
    File,
    Open,
    Close,
    Quit,
    Emulation,
    Pause,
    Resume,
    Reset,
    SaveStates,
    CreateSaveSlot,
    SaveActiveSlot,
    LoadActiveSlot,
    SelectActiveSlot,
    SaveSlot,
    LoadSlot,
    DeleteSlot,
    About,
    Settings,
    Preferences,
    General,
    Input,
    Video,
    Audio,
    System,
    Language,
    SaveStoragePolicy,
    SaveStorageDirectory,
    Browse,
    SystemDefault,
    Japanese,
    English,
    Sidecar,
    AppSharedData,
    CustomDirectory,
    PlayerOne,
    PlayerTwo,
    Shortcuts,
    CurrentBinding,
    Change,
    Clear,
    Unbound,
    FullscreenDefault,
    Scaling,
    Vsync,
    Mute,
    MasterVolume,
    SampleRate,
    AudioLatency,
    Filter,
    Mmc3IrqVariant,
    Auto,
    Sharp,
    Nec,
    None,
    NtscComposite,
    NtscSVideo,
    NtscRgb,
    FitToWindow,
    Ok,
    Cancel,
    ConflictDetected,
    CapturePrompt,
    InvalidCustomStorageDirectory,
    Debugger,
    OpenDebugger,
    StepFrame,
    StepInstr,
    PpuViewer,
    FollowPc,
    Go,
    Back,
    Disassembly,
    FollowPcTag,
    FixedTag,
    Memory,
    Space,
    Page,
    Prev,
    Next,
    AddrHex,
    SelectDumpRow,
    HexByte,
    Write,
    Watch,
    Freeze,
    Refresh,
    Confirm,
    Registers,
    Bookmarks,
    Label,
    Add,
    Set,
    PickRegister,
    Hex,
    WatchNone,
    FreezeNone,
    NoDisassembly,
    DisassemblyHint,
    NoImages,
    Panels,
    HoverTile,
    WriteNeedsSelection,
    FreezeNeedsSelection,
    WatchNeedsSelection,
    RegisterNeedsSelection,
    BackEmpty,
    BookmarkFull,
    FreezeCleared,
    NoCore,
    NothingToWrite,
    PausedState,
    RunningState,
}

pub fn resolve_language(language: AppLanguage) -> AppLanguage {
    match language {
        AppLanguage::SystemDefault => {
            let locale = std::env::var("LANG").unwrap_or_default().to_lowercase();
            if locale.starts_with("ja") {
                AppLanguage::Japanese
            } else {
                AppLanguage::English
            }
        }
        other => other,
    }
}

pub fn text(language: AppLanguage, key: UiText) -> &'static str {
    match resolve_language(language) {
        AppLanguage::Japanese => japanese_text(key),
        AppLanguage::English | AppLanguage::SystemDefault => english_text(key),
    }
}

fn english_text(key: UiText) -> &'static str {
    match key {
        UiText::File => "File",
        UiText::Open => "Open",
        UiText::Close => "Close",
        UiText::Quit => "Quit",
        UiText::Emulation => "Emulation",
        UiText::Pause => "Pause",
        UiText::Resume => "Resume",
        UiText::Reset => "Reset",
        UiText::SaveStates => "Save States",
        UiText::CreateSaveSlot => "Create Save Slot",
        UiText::SaveActiveSlot => "Save Active Slot",
        UiText::LoadActiveSlot => "Load Active Slot",
        UiText::SelectActiveSlot => "Select Active Slot",
        UiText::SaveSlot => "Save Slot",
        UiText::LoadSlot => "Load Slot",
        UiText::DeleteSlot => "Delete Slot",
        UiText::About => "About",
        UiText::Settings => "Settings",
        UiText::Preferences => "Preferences",
        UiText::General => "General",
        UiText::Input => "Input",
        UiText::Video => "Video",
        UiText::Audio => "Audio",
        UiText::System => "System",
        UiText::Language => "UI Language",
        UiText::SaveStoragePolicy => "Save storage policy",
        UiText::SaveStorageDirectory => "Save storage directory",
        UiText::Browse => "Browse",
        UiText::SystemDefault => "System default",
        UiText::Japanese => "日本語",
        UiText::English => "English",
        UiText::Sidecar => "Sidecar",
        UiText::AppSharedData => "App shared data",
        UiText::CustomDirectory => "Custom directory",
        UiText::PlayerOne => "Player 1",
        UiText::PlayerTwo => "Player 2",
        UiText::Shortcuts => "Shortcuts",
        UiText::CurrentBinding => "Current binding",
        UiText::Change => "Change",
        UiText::Clear => "Clear",
        UiText::Unbound => "Unbound",
        UiText::FullscreenDefault => "Fullscreen",
        UiText::Scaling => "Scaling",
        UiText::Vsync => "VSync",
        UiText::Mute => "Mute",
        UiText::MasterVolume => "Master volume",
        UiText::SampleRate => "Sample rate",
        UiText::AudioLatency => "Audio latency (ms)",
        UiText::Filter => "Filter",
        UiText::Mmc3IrqVariant => "MMC3 IRQ variant",
        UiText::Auto => "Auto",
        UiText::Sharp => "Sharp",
        UiText::Nec => "Nec",
        UiText::None => "None",
        UiText::NtscComposite => "NTSC Composite",
        UiText::NtscSVideo => "NTSC S-Video",
        UiText::NtscRgb => "NTSC RGB",
        UiText::FitToWindow => "Fit to window",
        UiText::Ok => "OK",
        UiText::Cancel => "Cancel",
        UiText::ConflictDetected => "Conflicting bindings must be resolved before saving.",
        UiText::CapturePrompt => "Press a key…",
        UiText::InvalidCustomStorageDirectory => {
            "The custom storage directory must exist or be creatable."
        }
        UiText::Debugger => "Debugger",
        UiText::OpenDebugger => "Open Debugger (F4)",
        UiText::StepFrame => "Step Frame",
        UiText::StepInstr => "Step Instr",
        UiText::PpuViewer => "PPU Viewer",
        UiText::FollowPc => "Follow PC",
        UiText::Go => "Go",
        UiText::Back => "Back",
        UiText::Disassembly => "Disassembly",
        UiText::FollowPcTag => "(follow PC)",
        UiText::FixedTag => "(fixed)",
        UiText::Memory => "Memory",
        UiText::Space => "space",
        UiText::Page => "page",
        UiText::Prev => "Prev",
        UiText::Next => "Next",
        UiText::AddrHex => "addr hex",
        UiText::SelectDumpRow => "Select a dump row",
        UiText::HexByte => "hex byte",
        UiText::Write => "Write",
        UiText::Watch => "Watch",
        UiText::Freeze => "Freeze",
        UiText::Refresh => "Refresh",
        UiText::Confirm => "Confirm",
        UiText::Registers => "Registers",
        UiText::Bookmarks => "Bookmarks",
        UiText::Label => "label",
        UiText::Add => "Add",
        UiText::Set => "Set",
        UiText::PickRegister => "Pick a register",
        UiText::Hex => "hex",
        UiText::WatchNone => "watch: none",
        UiText::FreezeNone => "freeze: none",
        UiText::NoDisassembly => "(no disassembly)",
        UiText::DisassemblyHint => "Enter an address and press Go, or press Back.",
        UiText::NoImages => "(no images)",
        UiText::Panels => "Panels",
        UiText::HoverTile => "hover a tile",
        UiText::WriteNeedsSelection => "write needs a selection",
        UiText::FreezeNeedsSelection => "freeze needs a selected row and hex byte",
        UiText::WatchNeedsSelection => "watch needs a selection",
        UiText::RegisterNeedsSelection => "register write needs a register",
        UiText::BackEmpty => "back stack is empty",
        UiText::BookmarkFull => "bookmark list full",
        UiText::FreezeCleared => "freeze cleared",
        UiText::NoCore => "no core",
        UiText::NothingToWrite => "nothing to write",
        UiText::PausedState => "paused",
        UiText::RunningState => "running",
    }
}

fn japanese_text(key: UiText) -> &'static str {
    match key {
        UiText::File => "ファイル",
        UiText::Open => "開く",
        UiText::Close => "閉じる",
        UiText::Quit => "終了",
        UiText::Emulation => "エミュレーション",
        UiText::Pause => "一時停止",
        UiText::Resume => "再開",
        UiText::Reset => "リセット",
        UiText::SaveStates => "セーブステート",
        UiText::CreateSaveSlot => "新しいスロットを作成",
        UiText::SaveActiveSlot => "アクティブスロットを保存",
        UiText::LoadActiveSlot => "アクティブスロットを読み込み",
        UiText::SelectActiveSlot => "アクティブスロットを選択",
        UiText::SaveSlot => "スロットを保存",
        UiText::LoadSlot => "スロットを読み込み",
        UiText::DeleteSlot => "スロットを削除",
        UiText::About => "情報",
        UiText::Settings => "設定",
        UiText::Preferences => "設定",
        UiText::General => "一般",
        UiText::Input => "入力",
        UiText::Video => "映像",
        UiText::Audio => "音声",
        UiText::System => "システム",
        UiText::Language => "UI 言語",
        UiText::SaveStoragePolicy => "保存先ポリシー",
        UiText::SaveStorageDirectory => "保存先ディレクトリ",
        UiText::Browse => "参照",
        UiText::SystemDefault => "システム既定",
        UiText::Japanese => "日本語",
        UiText::English => "English",
        UiText::Sidecar => "ROM と同じ場所",
        UiText::AppSharedData => "アプリ共通データ",
        UiText::CustomDirectory => "任意のフォルダ",
        UiText::PlayerOne => "プレイヤー 1",
        UiText::PlayerTwo => "プレイヤー 2",
        UiText::Shortcuts => "ショートカット",
        UiText::CurrentBinding => "現在の割当",
        UiText::Change => "変更",
        UiText::Clear => "解除",
        UiText::Unbound => "未割当",
        UiText::FullscreenDefault => "フルスクリーン",
        UiText::Scaling => "拡大率",
        UiText::Vsync => "VSync",
        UiText::Mute => "ミュート",
        UiText::MasterVolume => "主音量",
        UiText::SampleRate => "サンプルレート",
        UiText::AudioLatency => "音声遅延 (ms)",
        UiText::Filter => "フィルタ",
        UiText::Mmc3IrqVariant => "MMC3 IRQ バリアント",
        UiText::Auto => "自動",
        UiText::Sharp => "Sharp",
        UiText::Nec => "Nec",
        UiText::None => "なし",
        UiText::NtscComposite => "NTSC Composite",
        UiText::NtscSVideo => "NTSC S-Video",
        UiText::NtscRgb => "NTSC RGB",
        UiText::FitToWindow => "ウィンドウに合わせる",
        UiText::Ok => "OK",
        UiText::Cancel => "キャンセル",
        UiText::ConflictDetected => "割当競合を解消してください。",
        UiText::CapturePrompt => "キーを押してください…",
        UiText::InvalidCustomStorageDirectory => {
            "任意の保存先フォルダは存在するか作成可能である必要があります。"
        }
        UiText::Debugger => "デバッガ",
        UiText::OpenDebugger => "デバッガを開く (F4)",
        UiText::StepFrame => "フレーム実行",
        UiText::StepInstr => "命令実行",
        UiText::PpuViewer => "PPUビューア",
        UiText::FollowPc => "PC追従",
        UiText::Go => "移動",
        UiText::Back => "戻る",
        UiText::Disassembly => "逆アセンブル",
        UiText::FollowPcTag => "(PC追従)",
        UiText::FixedTag => "(固定)",
        UiText::Memory => "メモリ",
        UiText::Space => "スペース",
        UiText::Page => "ページ",
        UiText::Prev => "前へ",
        UiText::Next => "次へ",
        UiText::AddrHex => "アドレス16進",
        UiText::SelectDumpRow => "ダンプ行を選択",
        UiText::HexByte => "16進バイト",
        UiText::Write => "書込",
        UiText::Watch => "ウォッチ",
        UiText::Freeze => "フリーズ",
        UiText::Refresh => "更新",
        UiText::Confirm => "確定",
        UiText::Registers => "レジスタ",
        UiText::Bookmarks => "ブックマーク",
        UiText::Label => "ラベル",
        UiText::Add => "追加",
        UiText::Set => "設定",
        UiText::PickRegister => "レジスタを選択",
        UiText::Hex => "16進数",
        UiText::WatchNone => "ウォッチ: なし",
        UiText::FreezeNone => "フリーズ: なし",
        UiText::NoDisassembly => "(逆アセンブルなし)",
        UiText::DisassemblyHint => "アドレスを入力して「移動」、または「戻る」。",
        UiText::NoImages => "(画像なし)",
        UiText::Panels => "パネル",
        UiText::HoverTile => "タイルにホバー",
        UiText::WriteNeedsSelection => "書込には行選択が必要",
        UiText::FreezeNeedsSelection => "フリーズには行選択と16進バイトが必要",
        UiText::WatchNeedsSelection => "ウォッチには行選択が必要",
        UiText::RegisterNeedsSelection => "レジスタ書込にはレジスタ選択が必要",
        UiText::BackEmpty => "戻る履歴がありません",
        UiText::BookmarkFull => "ブックマークが上限です",
        UiText::FreezeCleared => "フリーズを解除しました",
        UiText::NoCore => "コアなし",
        UiText::NothingToWrite => "書込内容がありません",
        UiText::PausedState => "一時停止中",
        UiText::RunningState => "実行中",
    }
}

/// Parameterized debugger messages. `text()` covers fixed labels;
/// these carry addresses, values, and error detail, so one helper
/// per message keeps both languages side by side instead of
/// scattering `format!` across the frontends.
pub fn parse_failed(language: AppLanguage, input: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("解析失敗: {input}"),
        _ => format!("parse failed: {input}"),
    }
}

/// Selected dump row (`was ??` when the old byte is unreadable).
pub fn selected_row(language: AppLanguage, addr: u32, old: Option<u8>) -> String {
    let old_text = match old {
        Some(old) => format!("{old:02X}"),
        None => "??".to_string(),
    };
    match resolve_language(language) {
        AppLanguage::Japanese => format!("選択 {addr:08X} (元 {old_text})"),
        _ => format!("Selected {addr:08X} (was {old_text})"),
    }
}

/// Confirm-row text for a fully staged write.
pub fn pending_write(language: AppLanguage, addr: u32, old: Option<u8>, value: u64) -> String {
    let old_text = match old {
        Some(old) => format!("{old:02X}"),
        None => "??".to_string(),
    };
    match resolve_language(language) {
        AppLanguage::Japanese => format!("{addr:08X} に {value:02X} を書込 (元 {old_text})?"),
        _ => format!("write {value:02X} to {addr:08X} (was {old_text})?"),
    }
}

/// Committed single-byte memory write.
pub fn wrote_memory(language: AppLanguage, addr: u32, value: u64) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("{addr:08X} に {value:02X} を書込済 (幅1)"),
        _ => format!("wrote {value:02X} to {addr:08X} (width 1)"),
    }
}

/// Committed register write.
pub fn register_wrote(language: AppLanguage, name: &str, value: u64) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("レジスタ {name} に {value:04X} を書込済"),
        _ => format!("wrote {value:04X} to register {name}"),
    }
}

/// Domain-side refusal (`{detail:?}` at the call site).
pub fn write_refused(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("書込拒否: {detail}"),
        _ => format!("write refused: {detail}"),
    }
}

/// Domain-side register refusal.
pub fn register_refused(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("レジスタ書込拒否: {detail}"),
        _ => format!("register write refused: {detail}"),
    }
}

/// Emu-thread transport failure.
pub fn thread_failed(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("スレッドエラー: {detail}"),
        _ => format!("thread failed: {detail}"),
    }
}

/// Successful step (`stepped N`).
pub fn stepped(language: AppLanguage, count: u64) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("ステップ実行 {count}"),
        _ => format!("stepped {count}"),
    }
}

/// Failed step.
pub fn step_failed(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("ステップ失敗: {detail}"),
        _ => format!("step failed: {detail}"),
    }
}

/// Failed pause.
pub fn pause_failed(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("一時停止失敗: {detail}"),
        _ => format!("pause failed: {detail}"),
    }
}

/// Failed resume.
pub fn resume_failed(language: AppLanguage, detail: &str) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("再開失敗: {detail}"),
        _ => format!("resume failed: {detail}"),
    }
}

/// Applied freeze (`frozen {addr}={value}`).
pub fn frozen(language: AppLanguage, addr: u32, value: u8) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("フリーズ {addr:08X}={value:02X}"),
        _ => format!("frozen {addr:08X}={value:02X}"),
    }
}

/// Live watch readout (`= ??` when unreadable).
pub fn watch_value(language: AppLanguage, addr: u32, value: Option<u8>) -> String {
    let value_text = match value {
        Some(value) => format!("{value:02X}"),
        None => "??".to_string(),
    };
    match resolve_language(language) {
        AppLanguage::Japanese => format!("ウォッチ {addr:08X} = {value_text}"),
        _ => format!("watch {addr:08X} = {value_text}"),
    }
}

/// Tile hover readout (`+{offset:04X}` is the tile-start byte offset).
pub fn tile_hover(
    language: AppLanguage,
    label: &str,
    idx: u32,
    sx: u32,
    sy: u32,
    offset: u32,
) -> String {
    match resolve_language(language) {
        AppLanguage::Japanese => format!("{label} タイル #{idx} (px {sx},{sy}, +{offset:04X})"),
        _ => format!("{label} tile #{idx} (px {sx},{sy}, +{offset:04X})"),
    }
}
