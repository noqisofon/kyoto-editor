# kyoto-editor

京都とは全く関係ない [Koto](https://koto.dev) 言語で設定が書けるテキストエディタです。
キーバインドは CUA (Ctrl+C/X/V、Shift+矢印で選択 など)。

## 構成

| crate | 役割 |
| --- | --- |
| `kyoto-core` | 端末に依存しない部分。テキストバッファ、カーソル/選択、undo/redo、検索、コマンド、CUA キーマップ、Koto 設定ローダー |
| `kyoto-tui` | `kyoto` バイナリ。ratatui + crossterm による描画・入力 |

## 使い方

```sh
cargo run -p kyoto-tui -- [--config PATH] [FILE]

kyoto --print-config    # 設定ファイルの例を出力
kyoto --list-commands   # キーバインドに使えるコマンド名
kyoto --list-keys       # 既定の CUA キー割り当て
```

## キーバインド (既定 / CUA)

| キー | 動作 |
| --- | --- |
| Ctrl+C / X / V | コピー / 切り取り / 貼り付け (選択なしなら行単位) |
| Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z) | undo / redo |
| Ctrl+A | 全選択 |
| Ctrl+S / Ctrl+Shift+S | 保存 / 名前を付けて保存 |
| Ctrl+Q | 終了 (未保存なら 2 回押す) |
| Ctrl+F / F3 / Shift+F3 | 検索 / 次 / 前 (小文字のみの検索は大文字小文字を無視) |
| Ctrl+G | 行へ移動 |
| Shift+矢印/Home/End/PageUp/PageDown | 選択範囲を拡張 |
| Ctrl+←/→, Ctrl+Home/End | 単語移動 / 文書の先頭・末尾 |
| Ctrl+Backspace / Ctrl+Delete | 単語削除 |
| Tab / Shift+Tab | インデント / アウトデント (複数行選択に対応) |
| Alt+↑/↓ | 行の移動 |
| Ctrl+Shift+D / Ctrl+Shift+K | 行の複製 / 削除 |

コピーした内容は OSC 52 でシステムクリップボードにも送られます (対応端末のみ)。
Ctrl+Shift+… は端末が kitty キーボードプロトコルに対応している必要があります。

## Koto で書く設定

既定は `$KYOTO_CONFIG`、なければ `~/.config/kyoto/config.koto`
(`$XDG_CONFIG_HOME` を尊重)。`export` した値が設定として読まれ、それ以外は普通の Koto です。

```koto
export
  tab_width: 2
  insert_spaces: true
  line_numbers: true
  theme:
    selection_bg: '#264f78'
  keybindings:
    'ctrl+d': 'duplicate_line'      # コマンド名
    'ctrl+q': null                  # 割り当て解除
    'ctrl+shift+u': |ctx| ctx.selection.to_uppercase()   # Koto 関数
```

キーに関数を割り当てると、`ctx` (`selection`, `line`, `line_number`, `column`, `path`) を
受け取り、返した文字列で選択範囲を置き換えます (`null` なら何もしません)。
設定中の `kyoto.version` / `kyoto.commands` も参照できます。無限ループは 2 秒で打ち切られ、
`reload_config` コマンド (既定 Ctrl+R) で再読み込みできます。

## 開発

```sh
cargo test
```
