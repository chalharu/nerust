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
    NoSaveSlots,
    Help,
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
    RomLoadFailed,
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
        UiText::NoSaveSlots => "No save slots yet",
        UiText::Help => "Help",
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
        UiText::RomLoadFailed => "Failed to load ROM",
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
        UiText::NoSaveSlots => "セーブスロットがありません",
        UiText::Help => "ヘルプ",
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
        UiText::RomLoadFailed => "ROMの読み込みに失敗しました",
    }
}

/// Menu label with a shortcut hint (`Save Active Slot (F5)`).
/// One helper so every frontend advertises the same shortcuts
/// instead of drifting copies per shell.
pub fn menu_label_with_shortcut(label: &str, shortcut: &str) -> String {
    format!("{label} ({shortcut})")
}

/// Parameterized debugger messages. `text()` covers fixed labels;
/// these carry addresses, values, and error detail, so one helper
/// per message keeps both languages side by side instead of
/// scattering `format!` across the frontends.
///
/// Branch helper: one match instead of fifteen copies (arms stay
/// lazy like a hand-written match).
macro_rules! localized {
    ($language:expr, $en:expr, $ja:expr) => {
        match resolve_language($language) {
            AppLanguage::Japanese => $ja,
            _ => $en,
        }
    };
}

pub fn parse_failed(language: AppLanguage, input: &str) -> String {
    localized!(
        language,
        format!("parse failed: {input}"),
        format!("解析失敗: {input}")
    )
}

/// Selected dump row (`was ??` when the old byte is unreadable).
/// Addresses use the cross-system `0x` canon (never the NES `$`
/// sigil): the same debugger serves NES, GBC, and GBA.
pub fn selected_row(language: AppLanguage, addr: u32, old: Option<u8>) -> String {
    let old_text = match old {
        Some(old) => format!("{old:02X}"),
        None => "??".to_string(),
    };
    localized!(
        language,
        format!("Selected 0x{addr:08X} (was {old_text})"),
        format!("選択 0x{addr:08X} (元 {old_text})")
    )
}

/// Confirm-row text for a fully staged write.
pub fn pending_write(language: AppLanguage, addr: u32, old: Option<u8>, value: u64) -> String {
    let old_text = match old {
        Some(old) => format!("{old:02X}"),
        None => "??".to_string(),
    };
    localized!(
        language,
        format!("write {value:02X} to 0x{addr:08X} (was {old_text})?"),
        format!("0x{addr:08X} に {value:02X} を書込 (元 {old_text})?")
    )
}

/// Committed single-byte memory write.
pub fn wrote_memory(language: AppLanguage, addr: u32, value: u64) -> String {
    localized!(
        language,
        format!("wrote {value:02X} to 0x{addr:08X} (width 1)"),
        format!("0x{addr:08X} に {value:02X} を書込済 (幅1)")
    )
}

/// Committed register write.
pub fn register_wrote(language: AppLanguage, name: &str, value: u64) -> String {
    localized!(
        language,
        format!("wrote 0x{value:04X} to register {name}"),
        format!("レジスタ {name} に 0x{value:04X} を書込済")
    )
}

/// Domain-side refusal (`{detail:?}` at the call site).
pub fn write_refused(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("write refused: {detail}"),
        format!("書込拒否: {detail}")
    )
}

/// Domain-side register refusal.
pub fn register_refused(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("register write refused: {detail}"),
        format!("レジスタ書込拒否: {detail}")
    )
}

/// Emu-thread transport failure.
pub fn thread_failed(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("thread failed: {detail}"),
        format!("スレッドエラー: {detail}")
    )
}

/// Successful step (`stepped N`).
pub fn stepped(language: AppLanguage, count: u64) -> String {
    localized!(
        language,
        format!("stepped {count}"),
        format!("ステップ実行 {count}")
    )
}

/// Failed step.
pub fn step_failed(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("step failed: {detail}"),
        format!("ステップ失敗: {detail}")
    )
}

/// Failed pause.
pub fn pause_failed(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("pause failed: {detail}"),
        format!("一時停止失敗: {detail}")
    )
}

/// Failed resume.
pub fn resume_failed(language: AppLanguage, detail: &str) -> String {
    localized!(
        language,
        format!("resume failed: {detail}"),
        format!("再開失敗: {detail}")
    )
}

/// Applied freeze (`frozen {addr}={value}`).
pub fn frozen(language: AppLanguage, addr: u32, value: u8) -> String {
    localized!(
        language,
        format!("frozen 0x{addr:08X}={value:02X}"),
        format!("フリーズ 0x{addr:08X}={value:02X}")
    )
}

/// Live watch readout (`= ??` when unreadable).
pub fn watch_value(language: AppLanguage, addr: u32, value: Option<u8>) -> String {
    let value_text = match value {
        Some(value) => format!("{value:02X}"),
        None => "??".to_string(),
    };
    localized!(
        language,
        format!("watch 0x{addr:08X} = {value_text}"),
        format!("ウォッチ 0x{addr:08X} = {value_text}")
    )
}

/// Tile hover readout (`+0x{offset}` is the tile-start byte offset).
pub fn tile_hover(
    language: AppLanguage,
    label: &str,
    idx: u32,
    sx: u32,
    sy: u32,
    offset: u32,
) -> String {
    localized!(
        language,
        format!("{label} tile #{idx} (px {sx},{sy}, +0x{offset:04X})"),
        format!("{label} タイル #{idx} (px {sx},{sy}, +0x{offset:04X})")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> AppLanguage {
        AppLanguage::English
    }

    fn ja() -> AppLanguage {
        AppLanguage::Japanese
    }

    #[test]
    fn new_keys_resolve_in_both_languages() {
        use UiText::*;
        for key in [
            OpenDebugger,
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
            NoSaveSlots,
            Help,
            RomLoadFailed,
        ] {
            assert!(!text(en(), key).is_empty());
            assert!(!text(ja(), key).is_empty());
            assert_ne!(text(en(), key), text(ja(), key));
        }
    }

    #[test]
    fn parameterized_messages_cover_both_languages() {
        assert_eq!(parse_failed(en(), "zz"), "parse failed: zz");
        assert_eq!(parse_failed(ja(), "zz"), "解析失敗: zz");
        assert_eq!(
            selected_row(en(), 0x80, Some(0xAB)),
            "Selected 0x00000080 (was AB)"
        );
        assert_eq!(
            selected_row(en(), 0x80, None),
            "Selected 0x00000080 (was ??)"
        );
        assert_eq!(
            selected_row(ja(), 0x80, Some(0xAB)),
            "選択 0x00000080 (元 AB)"
        );
        assert_eq!(
            pending_write(en(), 0x80, Some(0), 0xCC),
            "write CC to 0x00000080 (was 00)?"
        );
        assert_eq!(
            pending_write(ja(), 0x80, None, 0xCC),
            "0x00000080 に CC を書込 (元 ??)?"
        );
        assert_eq!(
            wrote_memory(en(), 0x10, 0x42),
            "wrote 42 to 0x00000010 (width 1)"
        );
        assert_eq!(
            wrote_memory(ja(), 0x10, 0x42),
            "0x00000010 に 42 を書込済 (幅1)"
        );
        assert_eq!(register_wrote(en(), "r7", 1), "wrote 0x0001 to register r7");
        assert_eq!(
            register_wrote(ja(), "r7", 1),
            "レジスタ r7 に 0x0001 を書込済"
        );
        assert_eq!(write_refused(en(), "X"), "write refused: X");
        assert_eq!(write_refused(ja(), "X"), "書込拒否: X");
        assert_eq!(register_refused(en(), "X"), "register write refused: X");
        assert_eq!(register_refused(ja(), "X"), "レジスタ書込拒否: X");
        assert_eq!(thread_failed(en(), "X"), "thread failed: X");
        assert_eq!(thread_failed(ja(), "X"), "スレッドエラー: X");
        assert_eq!(stepped(en(), 7), "stepped 7");
        assert_eq!(stepped(ja(), 7), "ステップ実行 7");
        assert_eq!(step_failed(en(), "X"), "step failed: X");
        assert_eq!(step_failed(ja(), "X"), "ステップ失敗: X");
        assert_eq!(pause_failed(en(), "X"), "pause failed: X");
        assert_eq!(pause_failed(ja(), "X"), "一時停止失敗: X");
        assert_eq!(resume_failed(en(), "X"), "resume failed: X");
        assert_eq!(resume_failed(ja(), "X"), "再開失敗: X");
        assert_eq!(frozen(en(), 0x10, 0xFF), "frozen 0x00000010=FF");
        assert_eq!(frozen(ja(), 0x10, 0xFF), "フリーズ 0x00000010=FF");
        assert_eq!(watch_value(en(), 0x10, Some(1)), "watch 0x00000010 = 01");
        assert_eq!(watch_value(en(), 0x10, None), "watch 0x00000010 = ??");
        assert_eq!(watch_value(ja(), 0x10, Some(1)), "ウォッチ 0x00000010 = 01");
        assert_eq!(
            tile_hover(en(), "Tiles", 21, 44, 13, 0x150),
            "Tiles tile #21 (px 44,13, +0x0150)"
        );
        assert_eq!(
            tile_hover(ja(), "Tiles", 21, 44, 13, 0x150),
            "Tiles タイル #21 (px 44,13, +0x0150)"
        );
    }
}
