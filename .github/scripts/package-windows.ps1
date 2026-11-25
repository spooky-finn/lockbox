$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$version = $env:RELEASE_VERSION
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'RELEASE_VERSION must be X.Y.Z' }
$hostTarget = (rustc -vV | Select-String '^host: ').ToString().Substring(6)
if ($hostTarget -ne 'x86_64-pc-windows-msvc') { throw "Unexpected target: $hostTarget" }

$stage = Join-Path $env:RUNNER_TEMP "lockbox-$version-windows"
$output = 'target/release-assets'
New-Item -ItemType Directory -Path $stage, $output -Force | Out-Null
$env:CGO_ENABLED = '0'
go -C tools/storj-issuer build -mod=readonly -trimpath '-ldflags=-s -w' -o (Join-Path $stage 'lockbox-storj-issuer.exe') .
if ($LASTEXITCODE -ne 0) { throw 'Storj issuer build failed' }
Copy-Item target/release/lockbox.exe $stage
Copy-Item assets/LICENSE-simple-icons.md, assets/DISCLAIMER-simple-icons.md $stage
# Some WebView2 configurations use a dynamic loader instead of the static loader.
if (Test-Path target/release/WebView2Loader.dll) {
    Copy-Item target/release/WebView2Loader.dll $stage
}
@"
Lockbox $version - Windows x64
Extract the entire archive and run lockbox.exe.
Requires Microsoft Edge WebView2 Evergreen Runtime:
https://developer.microsoft.com/microsoft-edge/webview2/
This build is unsigned; Windows SmartScreen may display a warning.
Browser autofill/native messaging integration is not supported on Windows yet.
"@ | Set-Content (Join-Path $stage README.txt) -Encoding utf8
Compress-Archive -Path "$stage/*" -DestinationPath "$output/lockbox-$version-x86_64-pc-windows-msvc.zip"
