#!/usr/bin/env bash
# WSL から Windows の画面を撮り、PNG を WSL 側のパスに書く。描画を変えたら、実機で動かして撮り、
# Read で画像を見て確かめる。
# usage: scripts/win-screenshot.sh <out.png> [x y width height]
#   範囲は仮想スクリーンの物理ピクセル（主モニタの左上が 0,0。左や上のモニタは負）。省略すると全モニタ。
set -euo pipefail
OUT=${1:?usage: scripts/win-screenshot.sh <out.png> [x y width height]}
shift
export PATH="$PATH:/mnt/c/Windows/System32:/mnt/c/Windows/System32/WindowsPowerShell/v1.0"

# PowerShell は WSL のパスに書けないので、Windows の一時フォルダに書いてから移す。
TMP_WIN=$(cmd.exe /c "echo %TEMP%" 2>/dev/null | tr -d '\r')\\shunsoku-shot-$$.png
RECT=${*:-}

powershell.exe -NoProfile -NonInteractive -Command "
\$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
# PowerShell は DPI 非対応だと座標を拡大率で割る。物理ピクセルで撮るため PMv2 にする。
Add-Type -Name Dpi -Namespace W -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetProcessDpiAwarenessContext(System.IntPtr v);'
[void][W.Dpi]::SetProcessDpiAwarenessContext([System.IntPtr]-4)
\$r = '$RECT'.Trim()
if (\$r) { \$p = \$r -split '\s+' | ForEach-Object { [int]\$_ }; \$b = New-Object System.Drawing.Rectangle \$p[0], \$p[1], \$p[2], \$p[3] }
else { \$b = [System.Windows.Forms.SystemInformation]::VirtualScreen }
\$bmp = New-Object System.Drawing.Bitmap \$b.Width, \$b.Height
\$g = [System.Drawing.Graphics]::FromImage(\$bmp)
\$g.CopyFromScreen(\$b.Location, [System.Drawing.Point]::Empty, \$b.Size)
\$bmp.Save('$TMP_WIN', [System.Drawing.Imaging.ImageFormat]::Png)
\"\$(\$b.X) \$(\$b.Y) \$(\$b.Width) \$(\$b.Height)\"
" | tr -d '\r' | sed 's/^/captured (x y w h): /'

mkdir -p "$(dirname "$OUT")"
mv "$(wslpath "$TMP_WIN")" "$OUT"
echo "saved: $OUT"
