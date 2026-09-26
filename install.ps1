<#
.SYNOPSIS
    Instala CICADA para el usuario actual (no pide permisos de administrador).

.DESCRIPTION
    Funciona en los dos casos:
      - dentro del repositorio, donde compila con cargo;
      - dentro del zip de la página de releases, donde ya trae los binarios.

    En ambos:
      - copia los programas a %LOCALAPPDATA%\Programs\CICADA;
      - crea accesos directos en el Escritorio y en el menú Inicio;
      - añade la CLI `cicada` al PATH del usuario;
      - asocia los archivos .cicada con CICADA Studio;
      - copia los ejemplos a Documentos\CICADA sin pisar los que ya existan.

    Para desinstalar: uninstall.ps1 (queda también en la carpeta instalada).
#>
param(
    # Usar los ejecutables ya compilados en target\release.
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$dest = Join-Path $env:LOCALAPPDATA 'Programs\CICADA'
$docs = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'CICADA'

function Step($text) { Write-Host "  - $text" }

Write-Host ''
Write-Host 'CICADA - instalación' -ForegroundColor Red
Write-Host ''

# El zip de releases trae los .exe al lado del script; el repositorio, no.
$prebuilt = Test-Path (Join-Path $root 'cicada-studio.exe')
$binaries = if ($prebuilt) { $root } else { Join-Path $root 'target\release' }

if (-not $prebuilt -and -not $SkipBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Hace falta Rust para compilar CICADA: https://rustup.rs'
    }
    Step 'Compilando (la primera vez tarda unos minutos)...'
    cargo build --release -p cicada-studio -p cicada-cli --manifest-path (Join-Path $root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'La compilación falló.' }
}

foreach ($exe in 'cicada-studio.exe', 'cicada.exe') {
    if (-not (Test-Path (Join-Path $binaries $exe))) {
        throw "Falta $exe en $binaries. Ejecuta el script sin -SkipBuild."
    }
}

# Si Studio está abierto, Windows no deja reemplazar el ejecutable.
if (Get-Process cicada-studio -ErrorAction SilentlyContinue) {
    throw 'Cierra CICADA Studio antes de instalar.'
}

Step "Copiando a $dest"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $binaries 'cicada-studio.exe') $dest -Force
Copy-Item (Join-Path $binaries 'cicada.exe') $dest -Force
$icon = if ($prebuilt) { Join-Path $root 'cicada.ico' } else { Join-Path $root 'crates\cicada-studio\assets\icon.ico' }
Copy-Item $icon (Join-Path $dest 'cicada.ico') -Force
Copy-Item (Join-Path $root 'uninstall.ps1') $dest -Force
$studio = Join-Path $dest 'cicada-studio.exe'

Step "Ejemplos en $docs"
New-Item -ItemType Directory -Force $docs | Out-Null
$examples = Join-Path $root 'examples'
Get-ChildItem $examples -Recurse -Filter *.cicada | ForEach-Object {
    # Se respeta la carpeta del disco de ejemplo, que importa su paleta.
    $rel = $_.FullName.Substring($examples.Length).Trim([char]92)
    $target = Join-Path $docs $rel
    $dir = Split-Path $target
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
    if (-not (Test-Path $target)) { Copy-Item $_.FullName $target }
}

Step 'Accesos directos en el Escritorio y el menú Inicio'
$shell = New-Object -ComObject WScript.Shell
foreach ($dir in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
    $link = $shell.CreateShortcut((Join-Path $dir 'CICADA Studio.lnk'))
    $link.TargetPath = $studio
    $link.WorkingDirectory = $docs
    $link.IconLocation = "$studio,0"
    $link.Description = 'CICADA Studio: música electronica escrita como código'
    $link.Save()
}

Step 'CLI `cicada` en el PATH del usuario'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $userPath) { $userPath = '' }
if (($userPath -split ';') -notcontains $dest) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$dest").TrimStart(';'), 'User')
}

Step 'Archivos .cicada se abren con CICADA Studio'
$classes = 'HKCU:\Software\Classes'
function Set-Default($key, $value) {
    if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
    Set-Item -Path $key -Value $value
}
Set-Default "$classes\.cicada" 'CICADA.Track'
Set-Default "$classes\CICADA.Track" 'Pieza de CICADA'
Set-Default "$classes\CICADA.Track\DefaultIcon" "`"$(Join-Path $dest 'cicada.ico')`""
Set-Default "$classes\CICADA.Track\shell\open\command" "`"$studio`" `"%1`""

# Avisa al Explorador para que muestre el icono nuevo sin reiniciar.
Add-Type -Namespace Cicada -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll")]
public static extern void SHChangeNotify(int eventId, int flags, System.IntPtr item1, System.IntPtr item2);
'@
[Cicada.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

Write-Host ''
Write-Host 'Listo. Abre "CICADA Studio" desde el Escritorio.' -ForegroundColor Green
Write-Host 'La CLI `cicada` estará disponible en las terminales que abras a partir de ahora.'
Write-Host ''
