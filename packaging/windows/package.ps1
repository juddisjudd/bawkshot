# Packs target/release/bawkshot.exe into dist/ as a zip and an installer, each with its SHA-256 checksum.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path (Split-Path $PSScriptRoot))

$exe = "target/release/bawkshot.exe"
if (-not (Test-Path $exe)) { throw "Build first: cargo build --release" }
$iscc = Get-Command iscc -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
if (-not (Test-Path $iscc)) { throw "Inno Setup 6 was not found. Get it from https://jrsoftware.org/isdl.php" }

$version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages |
    Where-Object name -eq 'bawkshot' | Select-Object -ExpandProperty version
$name = "bawkshot-v$version-windows-x64"

if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory -Force "dist/$name" | Out-Null
Copy-Item $exe, LICENSE, README.md "dist/$name/"
Compress-Archive -Path "dist/$name" -DestinationPath "dist/$name.zip"
Remove-Item "dist/$name" -Recurse -Force

& $iscc /Q "/DAppVersion=$version" packaging/windows/bawkshot.iss
if ($LASTEXITCODE -ne 0) { throw "The installer did not compile" }

foreach ($file in Get-ChildItem dist -File) {
    $hash = (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLower()
    [IO.File]::WriteAllText("$($file.FullName).sha256", "$hash  $($file.Name)`n")
}
Get-ChildItem dist | ForEach-Object { Write-Output "dist/$($_.Name)" }
