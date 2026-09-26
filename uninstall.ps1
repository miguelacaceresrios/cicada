<#
.SYNOPSIS
    Desinstala CICADA del usuario actual. Tus piezas de Documentos\CICADA no se tocan.
#>

$ErrorActionPreference = 'Stop'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\CICADA'

if (Get-Process cicada-studio -ErrorAction SilentlyContinue) {
    throw 'Cierra CICADA Studio antes de desinstalar.'
}

foreach ($dir in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
    Remove-Item (Join-Path $dir 'CICADA Studio.lnk') -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $kept = ($userPath -split ';') | Where-Object { $_ -and $_ -ne $dest }
    [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
}

Remove-Item 'HKCU:\Software\Classes\.cicada' -Recurse -ErrorAction SilentlyContinue
Remove-Item 'HKCU:\Software\Classes\CICADA.Track' -Recurse -ErrorAction SilentlyContinue
Remove-Item $dest -Recurse -Force -ErrorAction SilentlyContinue

Write-Host 'CICADA desinstalado. Tus piezas siguen en Documentos\CICADA.' -ForegroundColor Green
