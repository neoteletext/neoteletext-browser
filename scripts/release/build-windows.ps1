# Release steps 1 and 2 on Windows (installation-and-updates.md sections
# 4.2 and 10), in the virtual machine, from the exported tree's native\
# folder: builds the release, signs every program with the code-signing
# certificate in its hardware module, lays out, lists and checks the
# version as the launcher will, and builds and signs NeoTeletext-Setup.exe
# carrying it.
#   powershell -File ..\scripts\release\build-windows.ps1 <version> <out> <certificate SHA-1 thumbprint> <timestamp server>
param([string]$Version, [string]$Out, [string]$Thumbprint, [string]$TimestampServer)
$ErrorActionPreference = "Stop"
$signtool = (Get-ChildItem -Recurse -Filter signtool.exe "C:\Program Files (x86)\Windows Kits\10\bin" | Where-Object { $_.FullName -like "*\x64\*" } | Select-Object -First 1).FullName
function Sign($files) {
    & $signtool sign /fd SHA256 /sha1 $Thumbprint /tr $TimestampServer /td SHA256 @files
    if ($LASTEXITCODE -ne 0) { throw "signtool" }
}
$programs = @("neoteletext", "neoteletext-decoder", "neoteletext-fetcher", "neoteletext-interface", "neoteletext-keyboard", "neoteletext-sound", "neoteletext-launcher", "neoteletext-stub")
$env:NEOTELETEXT_RELEASE_VERSION = $Version
$arguments = @("build", "--release", "--locked")
foreach ($program in $programs) { $arguments += @("-p", $program) }
& cargo @arguments
Push-Location tuf
cargo build --release --locked -p neoteletext-updater -p neoteletext-release
Pop-Location
Remove-Item Env:\NEOTELETEXT_RELEASE_VERSION
$release = "tuf\target\release\neoteletext-release.exe"
New-Item -ItemType Directory -Force "$Out\programs" | Out-Null
foreach ($program in $programs) { Copy-Item "target\release\$program.exe" "$Out\programs\" }
Copy-Item "tuf\target\release\neoteletext-updater.exe" "$Out\programs\"
$folder = "$Out\windows-x86_64\$Version"
& $release layout windows-x86_64 $Version "$Out\programs" $folder
Sign (Get-ChildItem "$folder\*.exe" | ForEach-Object { $_.FullName })
& $release manifest $folder windows-x86_64 $Version
& $release check $folder windows-x86_64
if ($LASTEXITCODE -ne 0) { throw "the launcher's check" }
$payload = "$Out\payload"
New-Item -ItemType Directory -Force $payload | Out-Null
Copy-Item "$Out\programs\neoteletext-stub.exe" "$payload\NeoTeletext.exe"
Sign @("$payload\NeoTeletext.exe")
Copy-Item "$folder\neoteletext-launcher.exe" "$payload\launcher.exe"
Copy-Item -Recurse $folder "$payload\$Version"
$env:NEOTELETEXT_INSTALLER_PAYLOAD = (Resolve-Path $payload).Path
cargo build --release --locked -p neoteletext-installer
Remove-Item Env:\NEOTELETEXT_INSTALLER_PAYLOAD
Copy-Item "target\release\neoteletext-installer.exe" "$Out\NeoTeletext-Setup.exe"
Sign @("$Out\NeoTeletext-Setup.exe")
"built and signed: $Out\NeoTeletext-Setup.exe; the update's folder is $folder"
