$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$assetsDirectory = Join-Path $PSScriptRoot '..\assets'
$svgPath = Join-Path $assetsDirectory 'app-icon.svg'
$previewPath = Join-Path $assetsDirectory 'app-icon.png'
$iconPath = Join-Path $assetsDirectory 'app-icon.ico'
$magickCommand = Get-Command magick.exe -ErrorAction SilentlyContinue
if (-not $magickCommand) {
    throw 'ImageMagick magick.exe is required to rasterize app-icon.svg.'
}

$tempDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ('exmonitor-icon-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempDirectory | Out-Null

try {
    $masterPath = Join-Path $tempDirectory 'master.png'
    & $magickCommand.Source -background none -density 1536 $svgPath -filter Lanczos -resize '1024x1024!' -filter Lanczos -resize '512x512!' -depth 8 -define png:color-type=6 -strip $masterPath
    if ($LASTEXITCODE -ne 0) {
        throw 'ImageMagick failed to render the SVG source.'
    }
    Copy-Item -LiteralPath $masterPath -Destination $previewPath -Force

    $sizes = @(16, 24, 32, 48, 64, 128, 256)
    $pngFrames = [System.Collections.Generic.List[byte[]]]::new()

    foreach ($size in $sizes) {
        $framePath = Join-Path $tempDirectory ("icon-{0}.png" -f $size)
        & $magickCommand.Source $masterPath -filter Lanczos -resize ("{0}x{0}!" -f $size) -depth 8 -define png:color-type=6 -strip $framePath
        if ($LASTEXITCODE -ne 0) {
            throw ("ImageMagick failed to render the {0}px icon." -f $size)
        }

        $image = [System.Drawing.Image]::FromFile($framePath)
        try {
            if ($image.Width -ne $size -or $image.Height -ne $size) {
                throw ("Incorrect frame size for {0}px icon." -f $size)
            }
        }
        finally {
            $image.Dispose()
        }

        $frame = [System.IO.File]::ReadAllBytes($framePath)
        $pngFrames.Add($frame) | Out-Null
    }

    $iconStream = [System.IO.File]::Open($iconPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write)
    try {
        $writer = [System.IO.BinaryWriter]::new($iconStream)
        $writer.Write([UInt16]0)
        $writer.Write([UInt16]1)
        $writer.Write([UInt16]$sizes.Count)
        $payloadOffset = 6 + ($sizes.Count * 16)
        $payloadBytes = 0

        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $size = $sizes[$index]
            $frame = $pngFrames[$index]
            $dimension = if ($size -eq 256) { [byte]0 } else { [byte]$size }
            $writer.Write($dimension)
            $writer.Write($dimension)
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([UInt16]1)
            $writer.Write([UInt16]32)
            $writer.Write([UInt32]$frame.Length)
            $writer.Write([UInt32]($payloadOffset + $payloadBytes))
            $payloadBytes += $frame.Length
        }

        foreach ($frame in $pngFrames) {
            $writer.Write($frame)
        }
        $writer.Flush()
    }
    finally {
        $iconStream.Dispose()
    }

    $verifyStream = [System.IO.File]::OpenRead($iconPath)
    try {
        $reader = [System.IO.BinaryReader]::new($verifyStream)
        $reserved = $reader.ReadUInt16()
        $iconType = $reader.ReadUInt16()
        $count = $reader.ReadUInt16()
        if ($reserved -ne 0 -or $iconType -ne 1 -or $count -ne $sizes.Count) {
            throw 'ICO header validation failed.'
        }

        $entries = for ($index = 0; $index -lt $count; $index++) {
            $widthByte = $reader.ReadByte()
            $heightByte = $reader.ReadByte()
            $reader.ReadByte() | Out-Null
            $reader.ReadByte() | Out-Null
            $reader.ReadUInt16() | Out-Null
            $bitsPerPixel = $reader.ReadUInt16()
            $payloadLength = $reader.ReadUInt32()
            $payloadOffset = $reader.ReadUInt32()
            $width = if ($widthByte -eq 0) { 256 } else { $widthByte }
            $height = if ($heightByte -eq 0) { 256 } else { $heightByte }

            if ($width -ne $height -or $bitsPerPixel -ne 32 -or $payloadLength -lt 24 -or $payloadOffset + $payloadLength -gt $verifyStream.Length) {
                throw 'ICO entry validation failed.'
            }

            [pscustomobject]@{
                Width = $width
                Height = $height
                BitsPerPixel = $bitsPerPixel
                PayloadBytes = $payloadLength
            }
        }

        if ((@($entries | Select-Object -ExpandProperty Width) -join ',') -ne ($sizes -join ',')) {
            throw 'ICO dimensions validation failed.'
        }
    }
    finally {
        $verifyStream.Dispose()
    }

    $preview = [System.Drawing.Image]::FromFile($previewPath)
    try {
        if ($preview.Width -ne 512 -or $preview.Height -ne 512) {
            throw 'Preview image is not 512x512.'
        }
        $bitmap = [System.Drawing.Bitmap]::new($preview)
        try {
            if ($bitmap.GetPixel(0, 0).A -ne 0) {
                throw 'Preview corners must remain transparent.'
            }
        }
        finally {
            $bitmap.Dispose()
        }
    }
    finally {
        $preview.Dispose()
    }

    Write-Output 'ICO entries:'
    $entries | Format-Table -AutoSize | Out-String | Write-Output
    Write-Output ("PNG preview: {0}" -f $previewPath)
    Write-Output ("ICO file: {0}" -f $iconPath)
}
finally {
    Remove-Item -LiteralPath $tempDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
