use crate::model::language::AppLanguage;

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
    NtscComposite,
    NtscSVideo,
    NtscRgb,
    FitToWindow,
    Ok,
    Save,
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
    Error,
    NotificationHistory,
    Notifications,
    NotificationInfo,
    NotificationWarning,
    NotificationError,
    NotificationStatus,
    NotificationDuration,
    NotificationColor,
    NotificationPosition,
    NotificationStatusPosition,
    NotificationAreaMode,
    NotificationStatusAreaMode,
    NotificationStatusAreaFollows,
    NotificationErrorModal,
    NotificationEnabled,
    NotificationErrorOnScreen,
    NotificationSectionDisplay,
    NotificationSectionStatus,
    HistoryFilterAll,
    HistoryLevel,
    HistoryClear,
    HistoryDatetime,
    HistoryModule,
    HistoryMessage,
    HistoryEmpty,
    ColorWhite,
    ColorYellow,
    ColorRed,
    ColorGreen,
    ColorCyan,
    PosTopLeft,
    PosTopRight,
    PosBottomLeft,
    PosBottomRight,
    AreaOverlay,
    AreaStrip,
    CrashTitle,
    CrashBody,
    CrashDetails,
    Latency,
    Discard,
    KeepEditing,
    DiscardUnsavedChanges,
    On,
    Off,
    Always,
    Hidden,
    NoTouchScreen,
    InputDeviceRequired,
    ScreenOrientation,
    AutoRotate,
    Portrait,
    Landscape,
    TouchOverlay,
    OverlayOpacity,
    OverlayScale,
    OverlayVerticalOffset,
    OverlayHaptics,
    CartridgeMotion,
    CartridgeRumble,
    RumbleStrength,
    RumbleTarget,
    Handset,
    Controller,
    Controls,
    Storage,
    Unknown,
    NotAssigned,
    OverlayDetail,
    ConnectedGamepads,
    DescVsync,
    DescOrientation,
    DescMute,
    DescSampleRate,
    DescLatency,
    DescNotifInfo,
    DescNotifWarning,
    DescNotifError,
    DescNotifErrorModal,
    DescNotifArea,
    DescStatusArea,
    DescOverlayVisibility,
    DescCartridgeMotion,
    DescCartridgeRumble,
    DescOverlayDetail,
    DescAudioDetail,
    DescShortcutsLink,
    DescStoragePolicy,
    SectionPlay,
    SectionControllers,
    SectionSaves,
    SectionApp,
    SectionAdvanced,
    Devices,
    ControllerType,
    NoKeyboards,
    DescNoKeyboards,
    ResetSettings,
    ResetScreen,
    Version,
    License,
    BuiltInKeyboard,
    ExternalKeyboard,
    GamepadDevice,
    AlwaysAvailable,
    Connected,
    SharesAcrossPlayers,
    Keyboard,
    AudioDetail,
    KeyMapping,
    Connection,
    Capabilities,
    KeyMappingDesc,
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
    let (english, japanese) = localized_pair(key);
    match resolve_language(language) {
        AppLanguage::Japanese => japanese,
        AppLanguage::English | AppLanguage::SystemDefault => english,
    }
}

/// CJK-capable sans family for the UI language, when the platform default cannot cover it.
/// `None` means the frontend default already covers the language.
pub fn font_family(language: AppLanguage) -> Option<&'static str> {
    if !matches!(resolve_language(language), AppLanguage::Japanese) {
        return None;
    }
    platform_cjk_sans()
}

#[cfg(target_os = "windows")]
fn platform_cjk_sans() -> Option<&'static str> {
    Some("Yu Gothic UI")
}

#[cfg(target_os = "macos")]
fn platform_cjk_sans() -> Option<&'static str> {
    Some("Hiragino Sans")
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_cjk_sans() -> Option<&'static str> {
    None
}

/// (English, Japanese) pair per key: one match instead of two
/// parallel 120-arm tables, so the languages can never drift apart.
fn localized_pair(key: UiText) -> (&'static str, &'static str) {
    match key {
        UiText::File => ("File", "ファイル"),
        UiText::Open => ("Open", "開く"),
        UiText::Close => ("Close", "閉じる"),
        UiText::Quit => ("Quit", "終了"),
        UiText::Emulation => ("Emulation", "エミュレーション"),
        UiText::Pause => ("Pause", "一時停止"),
        UiText::Resume => ("Resume", "再開"),
        UiText::Reset => ("Reset", "リセット"),
        UiText::SaveStates => ("Save States", "セーブステート"),
        UiText::CreateSaveSlot => ("Create Save Slot", "新しいスロットを作成"),
        UiText::SaveActiveSlot => ("Save Active Slot", "アクティブスロットを保存"),
        UiText::LoadActiveSlot => ("Load Active Slot", "アクティブスロットを読み込み"),
        UiText::SelectActiveSlot => ("Select Active Slot", "アクティブスロットを選択"),
        UiText::SaveSlot => ("Save Slot", "スロットを保存"),
        UiText::LoadSlot => ("Load Slot", "スロットを読み込み"),
        UiText::DeleteSlot => ("Delete Slot", "スロットを削除"),
        UiText::NoSaveSlots => ("No save slots yet", "セーブスロットがありません"),
        UiText::Help => ("Help", "ヘルプ"),
        UiText::About => ("About", "情報"),
        UiText::Settings => ("Settings", "設定"),
        UiText::Preferences => ("Preferences", "設定"),
        UiText::General => ("General", "一般"),
        UiText::Input => ("Input", "入力"),
        UiText::Video => ("Video", "映像"),
        UiText::Audio => ("Audio", "音声"),
        UiText::System => ("System", "システム"),
        UiText::Language => ("UI Language", "UI 言語"),
        UiText::SaveStoragePolicy => ("Save storage policy", "保存先ポリシー"),
        UiText::SaveStorageDirectory => ("Save storage directory", "保存先ディレクトリ"),
        UiText::Browse => ("Browse", "参照"),
        UiText::SystemDefault => ("System default", "システム既定"),
        UiText::Japanese => ("日本語", "日本語"),
        UiText::English => ("English", "English"),
        UiText::Sidecar => ("Sidecar", "ROM と同じ場所"),
        UiText::AppSharedData => ("App shared data", "アプリ共通データ"),
        UiText::CustomDirectory => ("Custom directory", "任意のフォルダ"),
        UiText::PlayerOne => ("Player 1", "プレイヤー 1"),
        UiText::PlayerTwo => ("Player 2", "プレイヤー 2"),
        UiText::Shortcuts => ("Shortcuts", "ショートカット"),
        UiText::CurrentBinding => ("Current binding", "現在の割当"),
        UiText::Change => ("Change", "変更"),
        UiText::Clear => ("Clear", "解除"),
        UiText::Unbound => ("Unbound", "未割当"),
        UiText::FullscreenDefault => ("Fullscreen", "フルスクリーン"),
        UiText::Scaling => ("Scaling", "拡大率"),
        UiText::Vsync => ("VSync", "VSync"),
        UiText::Mute => ("Mute", "ミュート"),
        UiText::MasterVolume => ("Master volume", "主音量"),
        UiText::SampleRate => ("Sample rate", "サンプルレート"),
        UiText::AudioLatency => ("Audio latency (ms)", "音声遅延 (ms)"),
        UiText::Filter => ("Filter", "フィルタ"),
        UiText::Mmc3IrqVariant => ("MMC3 IRQ variant", "MMC3 IRQ バリアント"),
        UiText::Auto => ("Auto", "自動"),
        UiText::Sharp => ("Sharp", "Sharp"),
        UiText::Nec => ("Nec", "Nec"),
        UiText::NtscComposite => ("NTSC Composite", "NTSC Composite"),
        UiText::NtscSVideo => ("NTSC S-Video", "NTSC S-Video"),
        UiText::NtscRgb => ("NTSC RGB", "NTSC RGB"),
        UiText::FitToWindow => ("Fit to window", "ウィンドウに合わせる"),
        UiText::Ok => ("OK", "OK"),
        UiText::Save => ("Save", "保存"),
        UiText::Cancel => ("Cancel", "キャンセル"),
        UiText::ConflictDetected => (
            "Conflicting bindings must be resolved before saving.",
            "重複したキー割り当てを解消してください。",
        ),
        UiText::CapturePrompt => ("Press a key…", "キーを押してください…"),
        UiText::InvalidCustomStorageDirectory => (
            "The custom storage directory must exist or be creatable.",
            "任意の保存先フォルダは存在するか作成可能である必要があります。",
        ),
        UiText::Debugger => ("Debugger", "デバッガ"),
        UiText::OpenDebugger => ("Open Debugger", "デバッガを開く"),
        UiText::StepFrame => ("Step Frame", "フレーム実行"),
        UiText::StepInstr => ("Step Instr", "命令実行"),
        UiText::PpuViewer => ("PPU Viewer", "PPUビューア"),
        UiText::FollowPc => ("Follow PC", "PC追従"),
        UiText::Go => ("Go", "移動"),
        UiText::Back => ("Back", "戻る"),
        UiText::Disassembly => ("Disassembly", "逆アセンブル"),
        UiText::FollowPcTag => ("(follow PC)", "(PC追従)"),
        UiText::FixedTag => ("(fixed)", "(固定)"),
        UiText::Memory => ("Memory", "メモリ"),
        UiText::Space => ("space", "スペース"),
        UiText::Page => ("page", "ページ"),
        UiText::Prev => ("Prev", "前へ"),
        UiText::Next => ("Next", "次へ"),
        UiText::AddrHex => ("addr hex", "アドレス16進"),
        UiText::SelectDumpRow => ("Select a dump row", "ダンプ行を選択"),
        UiText::HexByte => ("hex byte", "16進バイト"),
        UiText::Write => ("Write", "書込"),
        UiText::Watch => ("Watch", "ウォッチ"),
        UiText::Freeze => ("Freeze", "フリーズ"),
        UiText::Refresh => ("Refresh", "更新"),
        UiText::Confirm => ("Confirm", "確定"),
        UiText::Registers => ("Registers", "レジスタ"),
        UiText::Bookmarks => ("Bookmarks", "ブックマーク"),
        UiText::Label => ("label", "ラベル"),
        UiText::Add => ("Add", "追加"),
        UiText::Set => ("Set", "設定"),
        UiText::PickRegister => ("Pick a register", "レジスタを選択"),
        UiText::Hex => ("hex", "16進数"),
        UiText::WatchNone => ("watch: none", "ウォッチ: なし"),
        UiText::FreezeNone => ("freeze: none", "フリーズ: なし"),
        UiText::NoDisassembly => ("(no disassembly)", "(逆アセンブルなし)"),
        UiText::DisassemblyHint => (
            "Enter an address and press Go, or press Back.",
            "アドレスを入力して「移動」、または「戻る」。",
        ),
        UiText::NoImages => ("(no images)", "(画像なし)"),
        UiText::Panels => ("Panels", "パネル"),
        UiText::HoverTile => ("hover a tile", "タイルにホバー"),
        UiText::WriteNeedsSelection => ("write needs a selection", "書込には行選択が必要"),
        UiText::FreezeNeedsSelection => (
            "freeze needs a selected row and hex byte",
            "フリーズには行選択と16進バイトが必要",
        ),
        UiText::WatchNeedsSelection => ("watch needs a selection", "ウォッチには行選択が必要"),
        UiText::RegisterNeedsSelection => (
            "register write needs a register",
            "レジスタ書込にはレジスタ選択が必要",
        ),
        UiText::BackEmpty => ("back stack is empty", "戻る履歴がありません"),
        UiText::BookmarkFull => ("bookmark list full", "ブックマークが上限です"),
        UiText::FreezeCleared => ("freeze cleared", "フリーズを解除しました"),
        UiText::NoCore => ("no core", "コアなし"),
        UiText::NothingToWrite => ("nothing to write", "書込内容がありません"),
        UiText::PausedState => ("paused", "一時停止中"),
        UiText::RunningState => ("running", "実行中"),
        UiText::RomLoadFailed => ("Failed to load ROM", "ROMの読み込みに失敗しました"),
        UiText::Error => ("Error", "エラー"),
        UiText::NotificationHistory => ("Notification history", "通知履歴"),
        UiText::Notifications => ("Notifications", "通知"),
        UiText::NotificationInfo => ("Info", "情報"),
        UiText::NotificationWarning => ("Warning", "警告"),
        UiText::NotificationError => ("Error notifications", "エラー通知"),
        UiText::NotificationStatus => ("Status display", "ステータス表示"),
        UiText::NotificationDuration => ("Duration (ms)", "表示時間 (ms)"),
        UiText::NotificationColor => ("Color", "色"),
        UiText::NotificationPosition => ("Position", "表示位置"),
        UiText::NotificationStatusPosition => ("Status position", "ステータス位置"),
        UiText::NotificationAreaMode => ("Area", "表示領域"),
        UiText::NotificationStatusAreaMode => ("Status area", "ステータス領域"),
        UiText::NotificationStatusAreaFollows => (
            "Follows notification area (same edge)",
            "通知の領域に従う（同じ辺）",
        ),
        UiText::NotificationErrorModal => ("Modal dialog", "モーダルダイアログ"),
        UiText::NotificationEnabled => ("Enabled", "有効"),
        UiText::NotificationErrorOnScreen => ("On-screen display", "画面内表示"),
        UiText::NotificationSectionDisplay => ("Display", "表示"),
        UiText::NotificationSectionStatus => ("Status indicator", "状態表示"),
        UiText::HistoryFilterAll => ("All", "すべて"),
        UiText::HistoryLevel => ("Level", "レベル"),
        UiText::HistoryClear => ("Clear History", "履歴を消去"),
        UiText::HistoryDatetime => ("Datetime", "日時"),
        UiText::HistoryModule => ("Module", "モジュール"),
        UiText::HistoryMessage => ("Message", "メッセージ"),
        UiText::HistoryEmpty => ("No notifications", "通知はありません"),
        UiText::ColorWhite => ("White", "白"),
        UiText::ColorYellow => ("Yellow", "黄"),
        UiText::ColorRed => ("Red", "赤"),
        UiText::ColorGreen => ("Green", "緑"),
        UiText::ColorCyan => ("Cyan", "水色"),
        UiText::PosTopLeft => ("Top left", "左上"),
        UiText::PosTopRight => ("Top right", "右上"),
        UiText::PosBottomLeft => ("Bottom left", "左下"),
        UiText::PosBottomRight => ("Bottom right", "右下"),
        UiText::AreaOverlay => ("Overlay", "重ねる"),
        UiText::AreaStrip => ("Strip", "専用帯"),
        UiText::CrashTitle => ("nerust", "nerust"),
        UiText::CrashBody => (
            "The application crashed",
            "アプリケーションがクラッシュしました",
        ),
        UiText::CrashDetails => (
            "Details were written to the log (stderr).",
            "詳細はログ（stderr）に記録されました。",
        ),
        UiText::Latency => ("Latency", "遅延"),
        UiText::Discard => ("Discard", "破棄"),
        UiText::KeepEditing => ("Keep Editing", "編集を続ける"),
        UiText::DiscardUnsavedChanges => (
            "Discard unsaved changes?",
            "保存されていない変更を破棄しますか？",
        ),
        UiText::On => ("On", "オン"),
        UiText::Off => ("Off", "オフ"),
        UiText::Always => ("Always", "常に表示"),
        UiText::Hidden => ("Hidden", "非表示"),
        UiText::NoTouchScreen => ("No touch screen", "タッチ画面がありません"),
        UiText::InputDeviceRequired => ("Input device required", "入力デバイスが必要です"),
        UiText::ScreenOrientation => ("Screen orientation", "画面の向き"),
        UiText::AutoRotate => ("Auto rotate", "自動回転"),
        UiText::Portrait => ("Portrait", "縦向き"),
        UiText::Landscape => ("Landscape", "横向き"),
        UiText::TouchOverlay => ("Touch overlay", "タッチオーバーレイ"),
        UiText::OverlayOpacity => ("Overlay opacity", "オーバーレイ不透明度"),
        UiText::OverlayScale => ("Overlay scale", "オーバーレイ拡大率"),
        UiText::OverlayVerticalOffset => ("Overlay vertical position", "オーバーレイ垂直位置"),
        UiText::OverlayHaptics => ("Touch feedback", "タッチ操作時の振動"),
        UiText::CartridgeMotion => ("Cartridge motion sensor", "カートリッジモーションセンサー"),
        UiText::CartridgeRumble => ("Cartridge rumble", "カートリッジ振動"),
        UiText::RumbleStrength => ("Rumble strength", "振動の強さ"),
        UiText::RumbleTarget => ("Rumble target", "振動先"),
        UiText::Handset => ("Handset", "本体"),
        UiText::Controller => ("Controller", "コントローラー"),
        UiText::Controls => ("Controls", "操作"),
        UiText::Storage => ("Storage", "ストレージ"),
        UiText::Unknown => ("Unknown", "不明"),
        UiText::NotAssigned => ("Not assigned", "未割当"),
        UiText::OverlayDetail => ("Overlay detail", "オーバーレイ詳細"),
        UiText::DescVsync => ("", ""),
        UiText::DescOrientation => (
            "Applies immediately, even mid-game.",
            "ゲーム中も即時反映されます",
        ),
        UiText::DescMute => ("", ""),
        UiText::DescSampleRate => ("Adjust if audio crackles.", "音割れ対策用"),
        UiText::DescLatency => ("Less crackle, more delay.", "途切れ防止・遅延増加"),
        UiText::DescNotifInfo => ("Such as save confirmations.", "保存完了など"),
        UiText::DescNotifWarning => ("Slowdown or missing files.", "遅延・ファイル不足"),
        UiText::DescNotifError => ("Halted actions.", "操作停止"),
        UiText::DescNotifErrorModal => ("Separately from the overlay.", "オーバーレイと別に表示"),
        UiText::DescNotifArea => ("Overlay or reserved strip.", "重ね表示か専用帯"),
        UiText::DescStatusArea => (
            "A shared edge follows the message area.",
            "同じ辺は通知領域に従います",
        ),
        UiText::DescOverlayVisibility => (
            "Auto hides on gamepads; needs touch.",
            "パッド接続中は非表示。要タッチ画面",
        ),
        UiText::DescCartridgeMotion => ("Only some cartridges.", "一部カートリッジのみ使用します"),
        UiText::DescCartridgeRumble => (
            "Only cartridges with rumble use this.",
            "振動対応カートリッジのみ使用します",
        ),
        UiText::DescOverlayDetail => ("Size, position, transparency.", "大きさ・位置・透明度調整"),
        UiText::ConnectedGamepads => ("Connected gamepads", "接続中のゲームパッド"),
        UiText::DescAudioDetail => ("Sample rate, latency.", "サンプルレート・遅延"),
        UiText::DescShortcutsLink => ("Global save-state operations.", "セーブステート全体操作"),
        UiText::DescStoragePolicy => ("Takes effect on next launch.", "次回起動から有効です"),
        UiText::SectionPlay => ("Play", "プレイ"),
        UiText::SectionControllers => ("Controllers", "コントローラー"),
        UiText::SectionSaves => ("Saves", "保存データ"),
        UiText::SectionApp => ("App", "アプリ"),
        UiText::SectionAdvanced => ("Advanced", "詳細"),
        UiText::Devices => ("Devices", "デバイス"),
        UiText::ControllerType => ("Controller type", "コントローラー種類"),
        UiText::NoKeyboards => ("No keyboards connected", "接続キーボードなし"),
        UiText::DescNoKeyboards => (
            "Connect a USB or Bluetooth keyboard.",
            "USB・Bluetoothキーボードを接続",
        ),
        UiText::ResetSettings => ("Reset settings", "設定初期化"),
        UiText::ResetScreen => ("Reset this screen", "この画面初期化"),
        UiText::Version => ("Version", "バージョン"),
        UiText::License => ("License", "ライセンス"),
        UiText::BuiltInKeyboard => ("Built-in Keyboard", "内蔵キーボード"),
        UiText::ExternalKeyboard => ("External keyboard", "外付けキーボード"),
        UiText::GamepadDevice => ("Gamepad", "ゲームパッド"),
        UiText::AlwaysAvailable => ("Always available", "常に利用可能"),
        UiText::Connected => ("Connected", "接続中"),
        UiText::SharesAcrossPlayers => ("Shares across players", "複数プレイヤー共有可"),
        UiText::Keyboard => ("Keyboard", "キーボード"),
        UiText::AudioDetail => ("Audio details", "音声詳細"),
        UiText::KeyMapping => ("Key mapping", "キー割り当て"),
        UiText::Connection => ("Connection", "接続"),
        UiText::Capabilities => ("Capabilities", "対応機能"),
        UiText::KeyMappingDesc => ("", ""),
    }
}

/// Menu label with a shortcut hint (`Save Active Slot (F5)`).
/// One helper so every frontend advertises the same shortcuts.
pub fn menu_label_with_shortcut(label: &str, shortcut: &str) -> String {
    format!("{label} ({shortcut})")
}

/// Parameterized debugger messages: one helper per message keeps both
/// languages side by side instead of scattering `format!`.
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

/// History item count (`42 items` / `42件`). The number leads in
/// both languages (Console / Event Viewer pattern).
pub fn history_count(language: AppLanguage, count: usize) -> String {
    localized!(language, format!("{count} items"), format!("{count}件"))
}

/// Screen-reader label for a level enable toggle.
/// Pass the short level name; Error/Status read full elsewhere.
pub fn notification_group_label(language: AppLanguage, level: &str) -> String {
    localized!(
        language,
        format!("{level} notifications"),
        format!("{level}通知")
    )
}

/// Description for a controller-slot choice row: which port the row drives
/// is in the row label, so the description only states the action.
pub fn slot_description(language: AppLanguage) -> String {
    localized!(
        language,
        "Controller for this port.".to_string(),
        "このポートで使うコントローラー".to_string()
    )
}

/// Player/device-row summary: "Not assigned" when empty, otherwise
/// "Name" or "Name +N" (short form is banned: the full first name reads).
pub fn port_summary(language: AppLanguage, names: &[String]) -> String {
    match names {
        [] => text(language, UiText::NotAssigned).to_string(),
        [first] => first.clone(),
        // Count suffix is language-neutral, so no per-language branch.
        [first, rest @ ..] => format!("{first} +{}", rest.len()),
    }
}

/// Player/device-row count line: "No devices assigned" / "1 device
/// assigned" / "N devices assigned" (singular/plural distinguished).
pub fn port_count_line(language: AppLanguage, count: usize) -> String {
    match count {
        0 => localized!(
            language,
            "No devices assigned".to_string(),
            "割り当てデバイスなし".to_string()
        ),
        1 => localized!(
            language,
            "1 device assigned".to_string(),
            "1台のデバイスを割り当て".to_string()
        ),
        n => localized!(
            language,
            format!("{n} devices assigned"),
            format!("{n}台のデバイスを割り当て")
        ),
    }
}

/// Selected dump row (`was ??` when the old byte is unreadable).
/// Addresses use the cross-system `0x` canon (never the NES `$` sigil).
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
            Error,
            NotificationHistory,
            Notifications,
            NotificationInfo,
            NotificationWarning,
            NotificationError,
            NotificationStatus,
            NotificationDuration,
            NotificationColor,
            NotificationPosition,
            NotificationStatusPosition,
            NotificationAreaMode,
            NotificationStatusAreaMode,
            NotificationStatusAreaFollows,
            NotificationErrorModal,
            NotificationEnabled,
            NotificationErrorOnScreen,
            NotificationSectionDisplay,
            NotificationSectionStatus,
            HistoryFilterAll,
            HistoryLevel,
            HistoryClear,
            HistoryDatetime,
            HistoryModule,
            HistoryMessage,
            HistoryEmpty,
            ColorWhite,
            ColorYellow,
            ColorRed,
            ColorGreen,
            ColorCyan,
            PosTopLeft,
            PosTopRight,
            PosBottomLeft,
            PosBottomRight,
            AreaOverlay,
            AreaStrip,
            CrashBody,
            CrashDetails,
            Latency,
            Discard,
            KeepEditing,
            DiscardUnsavedChanges,
            Save,
            On,
            Off,
            Always,
            Hidden,
            NoTouchScreen,
            InputDeviceRequired,
            ScreenOrientation,
            AutoRotate,
            Portrait,
            Landscape,
            TouchOverlay,
            OverlayOpacity,
            OverlayScale,
            OverlayVerticalOffset,
            OverlayHaptics,
            CartridgeMotion,
            CartridgeRumble,
            RumbleStrength,
            RumbleTarget,
            Handset,
            Controller,
            Controls,
            Storage,
            Unknown,
            NotAssigned,
            OverlayDetail,
            DescOrientation,
            DescSampleRate,
            DescLatency,
            DescNotifInfo,
            DescNotifWarning,
            DescNotifError,
            DescNotifErrorModal,
            DescNotifArea,
            DescStatusArea,
            DescOverlayVisibility,
            DescCartridgeMotion,
            DescCartridgeRumble,
            DescOverlayDetail,
            ConnectedGamepads,
            DescAudioDetail,
            DescShortcutsLink,
            DescStoragePolicy,
            SectionPlay,
            SectionControllers,
            SectionSaves,
            SectionApp,
            SectionAdvanced,
            Devices,
            ControllerType,
            NoKeyboards,
            DescNoKeyboards,
            ResetSettings,
            ResetScreen,
            Version,
            License,
            BuiltInKeyboard,
            ExternalKeyboard,
            GamepadDevice,
            AlwaysAvailable,
            Connected,
            SharesAcrossPlayers,
            Keyboard,
            AudioDetail,
            KeyMapping,
            Connection,
            Capabilities,
        ] {
            assert!(!text(en(), key).is_empty());
            assert!(!text(ja(), key).is_empty());
            assert_ne!(text(en(), key), text(ja(), key));
        }
    }

    #[test]
    fn restating_descriptions_stay_bare() {
        // Self-evident labels carry no supporting text: the rows render
        // bare instead of restating the label.
        use UiText::*;
        for key in [DescVsync, DescMute, KeyMappingDesc] {
            assert!(text(en(), key).is_empty());
            assert!(text(ja(), key).is_empty());
        }
    }

    #[test]
    fn history_count_leads_with_the_number() {
        assert_eq!(history_count(en(), 6), "6 items");
        assert_eq!(history_count(ja(), 6), "6件");
        assert_eq!(history_count(en(), 0), "0 items");
    }

    #[test]
    fn notification_group_label_names_the_level() {
        assert_eq!(notification_group_label(en(), "Info"), "Info notifications");
        assert_eq!(notification_group_label(ja(), "情報"), "情報通知");
        assert_eq!(
            notification_group_label(en(), "Warning"),
            "Warning notifications"
        );
        assert_eq!(notification_group_label(ja(), "警告"), "警告通知");
        // Matches the existing full-name keys exactly.
        assert_eq!(
            notification_group_label(en(), text(en(), UiText::Error)),
            text(en(), UiText::NotificationError)
        );
        assert_eq!(
            notification_group_label(ja(), text(ja(), UiText::Error)),
            text(ja(), UiText::NotificationError)
        );
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

    #[test]
    fn cjk_font_family_follows_ui_language() {
        // English needs no override: the frontend default covers Latin.
        assert_eq!(font_family(en()), None);
        // Japanese needs an explicit CJK sans on Noto-less platforms;
        // Linux keeps the default (system fallback is consistent there).
        #[cfg(target_os = "windows")]
        assert_eq!(font_family(ja()), Some("Yu Gothic UI"));
        #[cfg(target_os = "macos")]
        assert_eq!(font_family(ja()), Some("Hiragino Sans"));
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(font_family(ja()), None);
    }
}
