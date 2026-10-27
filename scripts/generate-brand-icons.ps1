# Reproducible vector artwork: pirate sail above an audio wave.
$ErrorActionPreference = 'Stop'
$root = Join-Path (Split-Path $PSScriptRoot -Parent) 'assets/branding'
New-Item -ItemType Directory -Force $root | Out-Null
Add-Type -AssemblyName System.Drawing
$svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256"><rect width="256" height="256" rx="56" fill="#101c2b"/><path d="M125 40V166M125 46L60 146H125M139 62L194 146H139" fill="#f4dfaa" stroke="#f4dfaa" stroke-width="10" stroke-linejoin="round"/><path d="M149 38H190L180 50L190 62H149Z" fill="#ffb45c"/><path d="M40 185Q62 156 84 185T128 185T172 185T216 185M54 211Q76 190 98 211T142 211T186 211" fill="none" stroke="#56e0c2" stroke-width="12" stroke-linecap="round"/></svg>'
[IO.File]::WriteAllText((Join-Path $root 'piratewave.svg'), $svg)
$master = [Drawing.Bitmap]::new(1024,1024)
$g = [Drawing.Graphics]::FromImage($master)
$g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.ScaleTransform(4,4)
$bg = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#101c2b'))
$cream = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#f4dfaa'))
$gold = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#ffb45c'))
$mint = [Drawing.Pen]::new([Drawing.ColorTranslator]::FromHtml('#56e0c2'),12)
$mint.StartCap = $mint.EndCap = [Drawing.Drawing2D.LineCap]::Round
$shape=[Drawing.Drawing2D.GraphicsPath]::new()
foreach($arc in @(@(0,0,112,112,180,90),@(144,0,112,112,270,90),@(144,144,112,112,0,90),@(0,144,112,112,90,90))){$shape.AddArc($arc[0],$arc[1],$arc[2],$arc[3],$arc[4],$arc[5])}
$shape.CloseFigure(); $g.FillPath($bg,$shape)
$g.FillRectangle($cream,120,40,10,126)
$g.FillPolygon($cream,[Drawing.PointF[]]@([Drawing.PointF]::new(125,46),[Drawing.PointF]::new(60,146),[Drawing.PointF]::new(125,146)))
$g.FillPolygon($cream,[Drawing.PointF[]]@([Drawing.PointF]::new(139,62),[Drawing.PointF]::new(194,146),[Drawing.PointF]::new(139,146)))
$g.FillPolygon($gold,[Drawing.PointF[]]@([Drawing.PointF]::new(149,38),[Drawing.PointF]::new(190,38),[Drawing.PointF]::new(180,50),[Drawing.PointF]::new(190,62),[Drawing.PointF]::new(149,62)))
for($i=0;$i -lt 4;$i++){ $x=40+44*$i; $dy=if($i%2 -eq 0){-29}else{29}; $g.DrawBezier($mint,$x,185,($x+14.67),(185+$dy*0.667),($x+29.33),(185+$dy*0.667),($x+44),185) }
for($i=0;$i -lt 3;$i++){ $x=54+44*$i; $dy=if($i%2 -eq 0){-21}else{21}; $g.DrawBezier($mint,$x,211,($x+14.67),(211+$dy*0.667),($x+29.33),(211+$dy*0.667),($x+44),211) }
foreach($size in @(16,20,24,28,32,40,48,56,64,72,128,144,256,512)){
 $bitmap=[Drawing.Bitmap]::new($size,$size);$canvas=[Drawing.Graphics]::FromImage($bitmap)
 $canvas.InterpolationMode=[Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
 $canvas.DrawImage($master,0,0,$size,$size)
 $bitmap.Save((Join-Path $root "piratewave-$size.png"),[Drawing.Imaging.ImageFormat]::Png)
 $canvas.Dispose();$bitmap.Dispose()
}
$stream=[IO.File]::Create((Join-Path $root 'piratewave.ico'));$writer=[IO.BinaryWriter]::new($stream)
$sizes=@(16,24,32,48,64,128,256)
$writer.Write([uint16]0);$writer.Write([uint16]1);$writer.Write([uint16]$sizes.Count)
$offset=6+16*$sizes.Count
foreach($size in $sizes){$bytes=[IO.File]::ReadAllBytes((Join-Path $root "piratewave-$size.png"));$dimension=if($size -eq 256){0}else{$size};$writer.Write([byte]$dimension);$writer.Write([byte]$dimension);$writer.Write([uint16]0);$writer.Write([uint16]1);$writer.Write([uint16]32);$writer.Write([uint32]$bytes.Length);$writer.Write([uint32]$offset);$offset+=$bytes.Length}
foreach($size in $sizes){$writer.Write([IO.File]::ReadAllBytes((Join-Path $root "piratewave-$size.png")))}
$writer.Dispose();$g.Dispose();$master.Dispose();$shape.Dispose();$bg.Dispose();$cream.Dispose();$gold.Dispose();$mint.Dispose()
