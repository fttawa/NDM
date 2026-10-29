$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskCache = Join-Path $taskRoot '.native-build'
New-Item -ItemType Directory -Path $taskCache -Force | Out-Null

function Get-SourceHash($Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        return [System.BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
        $stream.Dispose()
    }
}

function Invoke-LoggedBuild($Program, $Arguments, $LogPath) {
    # Windows PowerShell converts redirected native stderr into ErrorRecords.
    # Keep diagnostics in the log and use the process exit code to detect failure.
    $ErrorActionPreference = 'Continue'
    & $Program @Arguments *> $LogPath
    if ($LASTEXITCODE -ne 0) {
        Get-Content -LiteralPath $LogPath -Tail 60
        throw "Native build failed ($Program, exit $LASTEXITCODE). See $LogPath"
    }
}

function Get-SourceArchive($Name, $Url, $Sha256) {
    $archive = Join-Path $taskCache $Name
    if (-not (Test-Path -LiteralPath $archive)) {
        Write-Host "Downloading $Name"
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $archive
    }
    if ((Get-SourceHash $archive) -ne $Sha256) {
        throw "Checksum mismatch: $Name. Remove the incomplete archive and retry."
    }
    return $archive
}

$taskVswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$taskVs = & $taskVswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $taskVs) { throw 'Install Visual Studio C++ Build Tools with the Windows SDK first.' }
Import-Module (Join-Path $taskVs 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $taskVs -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null
$env:VSLANG = '1033'
foreach ($taskToolDir in @('C:/Strawberry/perl/bin', 'C:/Strawberry/c/bin', (Join-Path $env:ProgramFiles 'NASM'))) {
    if (Test-Path -LiteralPath $taskToolDir) { $env:PATH = "$taskToolDir;$env:PATH" }
}
foreach ($tool in @('cmake', 'ninja', 'perl', 'nasm', 'nmake', 'git', 'tar')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Required native build tool missing: $tool" }
}

$taskBoost = Get-SourceArchive 'boost_1_88_0.tar.gz' 'https://archives.boost.io/release/1.88.0/source/boost_1_88_0.tar.gz' '3621533e820dcab1e8012afd583c0c73cf0f77694952b81352bf38c1488f9cb4'
if (-not (Test-Path (Join-Path $taskCache 'boost_1_88_0/boost/version.hpp'))) {
    & tar -xf $taskBoost -C $taskCache
    if ($LASTEXITCODE -ne 0) { throw 'Boost extraction failed' }
}
$taskSSL = Get-SourceArchive 'openssl-3.5.9.tar.gz' 'https://www.openssl.org/source/openssl-3.5.9.tar.gz' '603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a'
if (-not (Test-Path (Join-Path $taskCache 'openssl-3.5.9/Configure'))) {
    & tar -xf $taskSSL -C $taskCache
    if ($LASTEXITCODE -ne 0) { throw 'OpenSSL extraction failed' }
}
$taskSSLInstall = Join-Path $taskCache 'openssl-install'
if (-not (Test-Path (Join-Path $taskSSLInstall 'lib/libssl.lib'))) {
    Push-Location (Join-Path $taskCache 'openssl-3.5.9')
    try {
        Write-Output 'Building static OpenSSL (first build only)...'
        & perl Configure VC-WIN64A no-shared no-tests no-docs no-apps no-legacy no-module no-comp "--prefix=$($taskSSLInstall.Replace('\','/'))" "--openssldir=$($taskSSLInstall.Replace('\','/'))/ssl" /Os
        if ($LASTEXITCODE -ne 0) { throw 'OpenSSL configure failed' }
        Invoke-LoggedBuild 'nmake' @('/NOLOGO') (Join-Path $taskCache 'openssl-build.log')
        Invoke-LoggedBuild 'nmake' @('/NOLOGO', 'install_sw') (Join-Path $taskCache 'openssl-install.log')
    } finally { Pop-Location }
}

$taskLT = Join-Path $taskCache 'libtorrent'
$taskLTRevision = '56ae8caba38bf154ffc210403cb23f91d0ecaa49'
if (-not (Test-Path (Join-Path $taskLT '.git'))) {
    & git clone --depth 1 --branch v2.1.1 https://github.com/arvidn/libtorrent.git $taskLT
    if ($LASTEXITCODE -ne 0) { throw 'libtorrent source fetch failed' }
}
if ((& git -C $taskLT rev-parse HEAD) -ne $taskLTRevision) { throw 'Unexpected libtorrent source revision' }
& git -C $taskLT submodule update --init --depth 1 deps/try_signal
if ($LASTEXITCODE -ne 0) { throw 'libtorrent submodule fetch failed' }

$taskJSON = Join-Path $taskCache 'json/nlohmann/json.hpp'
New-Item -ItemType Directory -Path (Split-Path $taskJSON) -Force | Out-Null
if (-not (Test-Path -LiteralPath $taskJSON)) {
    Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/nlohmann/json/v3.12.0/single_include/nlohmann/json.hpp' -OutFile $taskJSON
}
if ((Get-SourceHash $taskJSON) -ne 'aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63') { throw 'JSON header checksum mismatch' }

$taskBuild = Join-Path $taskCache 'build'
# Strawberry Perl also ships GCC. Select MSVC explicitly so CMake does not
# choose that compiler ahead of the Visual Studio environment on clean runners.
$taskCompiler = (Get-Command cl.exe -ErrorAction Stop).Source.Replace('\', '/')
& cmake -S (Join-Path $taskRoot 'native/bt') -B $taskBuild -G Ninja "-DCMAKE_C_COMPILER=$taskCompiler" "-DCMAKE_CXX_COMPILER=$taskCompiler" -DCMAKE_BUILD_TYPE=MinSizeRel -DCMAKE_POLICY_DEFAULT_CMP0144=NEW "-DNDM_LIBTORRENT_SOURCE=$taskLT" "-DBOOST_ROOT=$(Join-Path $taskCache 'boost_1_88_0')" "-DOPENSSL_ROOT_DIR=$taskSSLInstall" "-DNDM_JSON_INCLUDE=$(Join-Path $taskCache 'json')"
if ($LASTEXITCODE -ne 0) { throw 'Native BT configuration failed' }
Write-Output 'Compiling native BT engine (see .native-build/native-build.log)...'
Invoke-LoggedBuild 'cmake' @('--build', $taskBuild, '--target', 'ndm-bt-engine', 'ndm-bt-fixture', '--parallel', '4') (Join-Path $taskCache 'native-build.log')
$taskOut = Join-Path $taskRoot 'src-tauri/bt-helper/native'
New-Item -ItemType Directory -Path $taskOut -Force | Out-Null
$taskBuiltEngine = Join-Path $taskBuild 'bin/ndm-bt-engine.exe'
$taskEngine = Join-Path $taskOut 'ndm-bt-engine.exe'
if (-not (Test-Path -LiteralPath $taskEngine) -or (Get-SourceHash $taskEngine) -ne (Get-SourceHash $taskBuiltEngine)) {
    Copy-Item -LiteralPath $taskBuiltEngine -Destination $taskEngine -Force
}
Write-Output "Native BT engine ready: $taskOut/ndm-bt-engine.exe"
