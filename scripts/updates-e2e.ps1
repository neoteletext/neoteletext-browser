# The end-to-end test of installation and updates on Windows
# (installation-and-updates.md section 13), run by hand in the virtual
# machine (DEVELOPMENT.md section 12.1) from the exported tree's native\
# folder, in an elevated ssh session: its certificate is trusted on the
# machine, and the updater's container is let reach the test host on this
# computer, both of which need an administrator (a per-user install never
# does). Windows with a window run in the logged-in desktop session, by a
# one-off interactive scheduled task.
#   powershell -ExecutionPolicy Bypass -File ..\scripts\updates-e2e.ps1
# It shows, with the real programs: NeoTeletext-Setup.exe's per-user
# install (the programs, the grant to ALL APPLICATION PACKAGES, the state,
# the Start menu shortcut and the uninstall entry); a healthy start; an
# update checked and staged by the confined updater, accepted by the
# pinned Authenticode signer, tried and committed; a release that crashes,
# rolled back after two starts; an unsigned release, refused; removal.
# Native programs write warnings on their error stream, which "Stop" would
# take for failures; each step checks its own outcome instead.
$ErrorActionPreference = "Continue"
$native = (Get-Location).Path
$work = "C:\build\nt-e2e"
$port = 8443
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$signtool = (Get-ChildItem -Recurse -Filter signtool.exe "C:\Program Files (x86)\Windows Kits\10\bin" | Where-Object { $_.FullName -like "*\x64\*" } | Select-Object -First 1).FullName
$root = Join-Path $env:LOCALAPPDATA "Programs\NeoTeletext"
$failed = $false

function Step($text) { "`n== $text" }
function Fail($text) { "FAILED: $text"; $script:failed = $true; throw $text }
function Ok($text) { "ok: $text" }

# Ends every program of the installation, whatever started it.
function Stop-Browser {
    Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith($root) } | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
}

# A run before may have left its installer showing why it failed, or a
# program of its versions running.
Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith($work) } | Stop-Process -Force -ErrorAction SilentlyContinue
if (Test-Path $root) { Stop-Browser; Remove-Item -Recurse -Force $root }
# Everything of a run before goes but its build folders, which only save time.
Get-ChildItem $work -ErrorAction SilentlyContinue | Where-Object { $_.Name -notin @("target", "tuf-target", "tools") } | Remove-Item -Recurse -Force
New-Item -ItemType Directory -Force $work | Out-Null
$identity = "$work\identity"
Remove-Item Env:\NEOTELETEXT_RELEASE_IDENTITY -ErrorAction SilentlyContinue

Step "the release tool, without the identity it makes"
Push-Location "$native\tuf"
cargo build --quiet --locked --offline --target-dir "$work\tools" -p neoteletext-release
Pop-Location
$tools = "$work\tools\debug"

Step "a self-signed code-signing certificate, trusted on this machine"
foreach ($store in @("Cert:\CurrentUser\My", "Cert:\LocalMachine\Root", "Cert:\LocalMachine\TrustedPublisher")) {
    Get-ChildItem $store | Where-Object { $_.Subject -eq "CN=NeoTeletext E2E Test" } | Remove-Item
}
$certificate = New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=NeoTeletext E2E Test" -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddDays(30)
Export-Certificate -Cert $certificate -FilePath "$work\sign.cer" | Out-Null
Import-Certificate -FilePath "$work\sign.cer" -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
Import-Certificate -FilePath "$work\sign.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPublisher | Out-Null
$thumb = $certificate.Thumbprint
function Sign($files) { & $signtool sign /q /fd SHA256 /sha1 $thumb @files; if ($LASTEXITCODE -ne 0) { Fail "signtool" } }
Copy-Item "$tools\neoteletext-release.exe" "$work\probe.exe"
Sign @("$work\probe.exe")
$signer = (& "$tools\neoteletext-release.exe" signer-key "$work\probe.exe").Trim()
"the signer's key: $signer"
& "$tools\neoteletext-release.exe" test-identity $identity --host localhost --port $port --windows-signer $signer
$env:NEOTELETEXT_RELEASE_IDENTITY = $identity
Push-Location "$native\tuf"
cargo build --quiet --locked --offline --target-dir "$work\tools" -p neoteletext-release -p neoteletext-updater --bins
Pop-Location
function Key($name) { "file:$identity\keys\$name.pk8" }

$programs = @("neoteletext", "neoteletext-decoder", "neoteletext-fetcher", "neoteletext-interface", "neoteletext-keyboard", "neoteletext-sound", "neoteletext-launcher")
function Build($version) {
    Step "the browser $version, built with the test identity"
    $env:NEOTELETEXT_RELEASE_VERSION = $version
    $arguments = @("build", "--quiet", "--release", "--locked", "--offline", "--target-dir", "$work\target")
    foreach ($program in $programs) { $arguments += @("-p", $program) }
    $arguments += @("-p", "neoteletext-stub")
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { Fail "build $version" }
    Push-Location "$native\tuf"
    cargo build --quiet --release --locked --offline --target-dir "$work\tuf-target" -p neoteletext-updater
    Pop-Location
    Remove-Item Env:\NEOTELETEXT_RELEASE_VERSION
    $folder = "$work\programs-$version"
    New-Item -ItemType Directory -Force $folder | Out-Null
    foreach ($program in $programs) { Copy-Item "$work\target\release\$program.exe" $folder }
    Copy-Item "$work\tuf-target\release\neoteletext-updater.exe" $folder
}

function Release($version, $how = "test") {
    Step "release ${version}: laid out, signed ($how), listed, published"
    $built = "$work\built\$version"
    & "$tools\neoteletext-release.exe" layout windows-x86_64 $version "$work\programs-$version" $built
    if ($how -eq "test") { Sign (Get-ChildItem "$built\*.exe" | ForEach-Object { $_.FullName }) }
    # The manifest, signed by the release key (decision 92), signed or not
    # by Authenticode.
    & "$tools\neoteletext-release.exe" sign-windows $built $version (Key linux-release)
    if ($how -eq "test") {
        & "$tools\neoteletext-release.exe" check $built windows-x86_64 --test-identity
        if ($LASTEXITCODE -ne 0) { Fail "the launcher's check of $version" }
    }
    & "$tools\neoteletext-release.exe" targets $built windows-x86_64 "$work\targets-$version"
    & "$tools\neoteletext-release.exe" publish "$work\repository" "$identity\root.json" "$work\targets-$version" --targets (Key targets) --snapshot (Key snapshot) --timestamp (Key timestamp)
    if ($LASTEXITCODE -ne 0) { Fail "publish $version" }
}

# Runs a command in the logged-in desktop session, waits, and ends the
# browser; what the command printed is returned.
function In-Desktop($command, $seconds) {
    $script = "$work\desktop.ps1"
    $out = "$work\desktop.txt"
    # "&" does not wait for a desktop program or take its output:
    # Start-Process hands it the file, which every process it starts
    # inherits.
    Set-Content $script "Start-Process -FilePath '$command' -Wait -RedirectStandardError '$out' -RedirectStandardOutput '$out.stdout'"
    Remove-Item $out, "$out.stdout" -ErrorAction SilentlyContinue
    # schtasks says on its error stream that a task is not there; that is
    # no failure here.
    cmd /c "schtasks /delete /tn NeoTeletextE2E /f >nul 2>&1"
    cmd /c "schtasks /create /f /tn NeoTeletextE2E /sc once /st 00:00 /it /rl limited /tr `"powershell -NoProfile -ExecutionPolicy Bypass -File $script`" >nul 2>&1"
    cmd /c "schtasks /run /tn NeoTeletextE2E >nul 2>&1"
    Start-Sleep -Seconds $seconds
    Stop-Browser
    cmd /c "schtasks /delete /tn NeoTeletextE2E /f >nul 2>&1"
    Get-Content $out -ErrorAction SilentlyContinue | Where-Object { $_ -match "^neoteletext" }
}

function State { Get-Content "$root\state" -Raw }
function Expect-State($what, $pattern) {
    $state = State
    if ($state -notmatch [regex]::Escape($pattern)) { $state; Fail "${what}: the state lacks '$pattern'" }
    Ok $what
}

# 1. The installer.
Build "0.1.0"
Release "0.1.0"
Step "NeoTeletext-Setup.exe, carrying release 0.1.0"
$payload = "$work\payload"
New-Item -ItemType Directory -Force $payload | Out-Null
Copy-Item "$work\target\release\neoteletext-stub.exe" "$payload\NeoTeletext.exe"
Copy-Item "$work\built\0.1.0\neoteletext-launcher.exe" "$payload\launcher.exe"
Sign @("$payload\NeoTeletext.exe")
Copy-Item -Recurse "$work\built\0.1.0" "$payload\0.1.0"
$env:NEOTELETEXT_INSTALLER_PAYLOAD = $payload
cargo build --quiet --release --locked --offline --target-dir "$work\target" -p neoteletext-installer
Remove-Item Env:\NEOTELETEXT_INSTALLER_PAYLOAD
Copy-Item "$work\target\release\neoteletext-installer.exe" "$work\NeoTeletext-Setup.exe"
Sign @("$work\NeoTeletext-Setup.exe")
"the stub: $((Get-Item "$payload\NeoTeletext.exe").Length) bytes"
In-Desktop "$work\NeoTeletext-Setup.exe" 25
foreach ($file in @("NeoTeletext.exe", "launcher-a.exe", "launcher", "state", "versions\0.1.0\neoteletext.exe", "versions\0.1.0\manifest")) {
    if (-not (Test-Path "$root\$file")) { Fail "the install lacks $file" }
}
Expect-State "1. installed" "current 0.1.0"
$acl = (Get-Acl "$root\versions").Access | Where-Object { $_.IdentityReference -like "*ALL APPLICATION PACKAGES*" }
if (-not $acl) { Fail "versions\ does not grant ALL APPLICATION PACKAGES" }
Ok "1. versions\ grants ALL APPLICATION PACKAGES $($acl.FileSystemRights)"
$link = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\NeoTeletext.lnk"
$target = (New-Object -ComObject WScript.Shell).CreateShortcut($link).TargetPath
if ($target -ne "$root\NeoTeletext.exe") { Fail "the shortcut leads to '$target'" }
Ok "1. the Start menu shortcut leads to the stub"
$entry = Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NeoTeletext"
if ($entry.UninstallString -ne "`"$root\NeoTeletext.exe`" --uninstall" -or $entry.DisplayVersion -ne "0.1.0") { Fail "the uninstall entry: $($entry | Out-String)" }
Ok "1. the uninstall entry"

# 2. An update.
Step "the test host on localhost:$port, and the updater's container let reach it"
$hostProcess = Start-Process -PassThru -WindowStyle Hidden -FilePath "$tools\neoteletext-test-host.exe" -ArgumentList "$work\repository", "$identity\host.pem", "$identity\host.key.pem", "localhost:$port"
Build "0.1.1"
Release "0.1.1"
# The container profile is made at the updater's first start.
& "$work\tools\debug\updater-drive.exe" "$root\versions\0.1.0\neoteletext-updater.exe" check 2>&1 | Out-Null
# CheckNetIsolation names a container made by CreateAppContainerProfile by
# its SID; it refuses the name ("Invalid Parameters").
$mappings = "HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings"
function Container-Sid($moniker) { (Get-ChildItem $mappings | Where-Object { (Get-ItemProperty $_.PSPath).Moniker -eq $moniker }).PSChildName }
$updaterSid = Container-Sid "neoteletext.updater"
if (-not $updaterSid) { Fail "the updater's container profile was not made" }
CheckNetIsolation.exe LoopbackExempt -a "-p=$updaterSid" | Out-Null
if ($LASTEXITCODE -ne 0) { Fail "the updater's container was not let reach the test host" }
Step "2. the confined updater checks and stages 0.1.1"
$drive = & "$work\tools\debug\updater-drive.exe" "$root\versions\0.1.0\neoteletext-updater.exe" stage 0.1.1 2>&1
$drive
if (-not (($drive | Out-String) -match "Staged\(")) { Fail "0.1.1 was not staged" }
In-Desktop "$root\NeoTeletext.exe" 60
Expect-State "2. the update committed" "current 0.1.1"
Expect-State "2. the old version kept" "previous 0.1.0"

# 3. A release that crashes.
Step "3. 0.1.2, whose broker crashes at start"
cargo build --quiet --release --locked --offline --target-dir "$work\target" -p neoteletext-launcher --bin launcher-test-broker
Copy-Item -Recurse "$work\programs-0.1.1" "$work\programs-0.1.2"
& "$work\target\release\launcher-test-broker.exe" --make 0.1.2 crash "$work\programs-0.1.2\neoteletext.exe"
if ($LASTEXITCODE -ne 0) { Fail "the broken broker was not made" }
Release "0.1.2"
& "$work\tools\debug\updater-drive.exe" "$root\versions\0.1.1\neoteletext-updater.exe" stage 0.1.2 2>&1 | Out-Null
In-Desktop "$root\NeoTeletext.exe" 40
Expect-State "3. rolled back" "current 0.1.1"
Expect-State "3. marked bad" "bad 0.1.2"

# 4. An unsigned release.
Build "0.1.3"
Release "0.1.3" "unsigned"
Step "4. 0.1.3, not signed"
& "$work\tools\debug\updater-drive.exe" "$root\versions\0.1.1\neoteletext-updater.exe" stage 0.1.3 2>&1 | Out-Null
In-Desktop "$root\NeoTeletext.exe" 30
# Refused, removed, and not marked bad: "bad" is a trial that failed
# (finding F-08).
if ((State) -match [regex]::Escape("bad 0.1.3")) { Fail "4. a refused version was marked bad" }
if (Test-Path "$root\staging\0.1.3") { Fail "4. the refused version was not removed" }
Ok "4. refused and removed, not marked bad"
Expect-State "4. the current version still runs" "current 0.1.1"

# 5. Removal, as the uninstaller's copy does it once the questions are
# answered (the questions themselves are message boxes, for a hand).
Step "5. removal"
Stop-Process -Id $hostProcess.Id -Force
Copy-Item "$root\launcher-a.exe" "$work\uninstall.exe"
# The copy waits for its standard input to close, as it waits for the
# launcher that started it to end; here an empty file. It ends by saying
# so in a message box, which nobody here can press: it is ended once the
# installation is gone.
Set-Content "$work\empty.txt" ""
$uninstaller = Start-Process -PassThru -WindowStyle Hidden -FilePath "$work\uninstall.exe" -ArgumentList "--uninstall-from", "`"$root`"", "--and-settings" -RedirectStandardInput "$work\empty.txt"
foreach ($second in 1..30) { if (-not (Test-Path $root)) { break }; Start-Sleep -Seconds 1 }
Start-Sleep -Seconds 3
Stop-Process -Id $uninstaller.Id -Force -ErrorAction SilentlyContinue
if (Test-Path $root) { Fail "the installation is still there" }
if (Test-Path $link) { Fail "the shortcut is still there" }
if (Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NeoTeletext") { Fail "the uninstall entry is still there" }
$left = Get-ChildItem $mappings | Where-Object { (Get-ItemProperty $_.PSPath).Moniker -like "neoteletext.*" }
if ($left) { Fail "the containers' profiles are still there: $($left.PSChildName)" }
Ok "5. removed, with the containers' profiles"
CheckNetIsolation.exe LoopbackExempt -d "-p=$updaterSid" | Out-Null
Remove-Item "Cert:\LocalMachine\Root\$thumb", "Cert:\LocalMachine\TrustedPublisher\$thumb", "Cert:\CurrentUser\My\$thumb" -ErrorAction SilentlyContinue
"`n== All end-to-end checks passed (Windows)"
