# nerust-keyboard crate 設計

## 動機

`fix/keyboard-code` (#315) では `keyboard_types::Code` に置き換えたが、以下の問題がある:

1. **プラットフォーム変換が Rust らしくない**: `keycode_controller_input()` など 4 つの変換関数が 50 行の match 文で残った。`From` trait があれば `key.into()` の 1 行になる
2. **外部依存**: `keyboard-types` crate が必要。serde の出力形式がバージョンで変わるリスク
3. **拡張性**: keyboard 以外の入力（gamepad など）との統合インタフェースが none

## 設計

### ディレクトリ構成

```
keyboard/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── tao.rs       #[cfg(feature = "tao")]
    ├── gtk.rs       #[cfg(feature = "gtk")]
    └── android.rs   #[cfg(feature = "android")]
```

### Cargo.toml

```toml
[package]
name = "nerust_keyboard"

[features]
default = ["tao", "gtk", "android"]
tao = ["dep:tao"]
gtk = ["dep:gdk4"]
android = ["dep:ndk"]

[dependencies]
serde = { ... }
serde-saphyr = { ... }

# Platform deps (optional, gated by features)
tao = { package = "tao", optional = true, version = "0.35" }
gdk4 = { package = "gdk4", optional = true, version = "0.11" }
ndk = { optional = true, version = "0.9" }
```

### コア型

```rust
// keyboard/src/lib.rs

/// キーボードキーを表す。W3C UI Events 準拠の variant 名を使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    KeyA, KeyB, ..., KeyZ,
    Digit0, ..., Digit9,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Enter, Escape, Space, Tab,
    F1, ..., F12,
}
```

### From 実装

```rust
// keyboard/src/tao.rs
#[cfg(feature = "tao")]
impl From<tao::keyboard::KeyCode> for Key {
    fn from(code: tao::keyboard::KeyCode) -> Self {
        match code {
            tao::keyboard::KeyCode::KeyA => Key::KeyA,
            // ... (機械的な 1:1 マッピング)
        }
    }
}

// keyboard/src/iced.rs — iced 依存は基本的にないが、
// 必要なら独自の From を追加可能
```

### serde 形式

現在の `KeyboardKey::KeyA` → `"key_a"` の YAML 互換性を維持:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    ...
}
```

これにより旧設定ファイルとの互換性が保たれる。

### 移行手順

1. `keyboard/` crate を作成、`Key` enum + `From<tao::KeyCode>` を実装
2. `Cargo.toml` に `nerust_keyboard` を追加
3. `gui/settings` の `KeyboardKey` → `nerust_keyboard::Key` に置き換え
4. 4 つの変換関数（`keycode_controller_input`, `keyboard_key_from_physical`, `gdk_key_controller_input`, `gdk_key_to_keyboard_key`）を削除し、代わりに `From` を使用
5. `fix/keyboard-code` (#315) の `keyboard-types` 依存を削除

### 削減見積もり

| 項目 | 現状 (#315) | 新設計 | 差 |
|---|---|---|---|
| `keyboard-types` 依存 | 必要 | 不要 | -1 crate |
| serde 制御 | 間接的（外部） | 直接的（自前） | 0 |
| tao `From` | match 52行 | impl 52行 | 0 |
| iced `From` | match 52行 | 不要（iced に `From` なし） | -52行 |
| gtk `From` | match 52行 × 2 | impl 52行 | -52行 |
| 設定互換性 | 変わる可能性あり | 維持 | - |

`iced` の `keyboard::key::Code` は `iced` crate の内部型であり `From` を実装できないため、`keyboard_key_from_physical()` の match 文は残る。ただし `iced` 依存を `nerust_keyboard` crate で持つのは重すぎるため、Tao frontend 側に match を 1 つ残すのは許容範囲。
