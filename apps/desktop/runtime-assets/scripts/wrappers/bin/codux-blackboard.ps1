$wrapperRoot = $PSScriptRoot
if (-not $wrapperRoot) { $wrapperRoot = Split-Path -Parent $MyInvocation.MyCommand.Path }
if ($wrapperRoot.StartsWith('\\?\UNC\')) { $wrapperRoot = '\\' + $wrapperRoot.Substring(8) }
elseif ($wrapperRoot.StartsWith('\\?\')) { $wrapperRoot = $wrapperRoot.Substring(4) }
$helper = Join-Path $wrapperRoot '..\codux-wrapper-helper.exe'
if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) {
  Write-Error 'codux-blackboard: bundled helper is missing'
  exit 127
}
$ErrorActionPreference = 'Stop'
$requestPath = Join-Path ([IO.Path]::GetTempPath()) "codux-launch-$([Guid]::NewGuid().ToString('N')).json"
try {
  $commandArgs = @('--codux-wrapper-helper', 'blackboard') + @($args | ForEach-Object { [string]$_ })
  $request = @{binary=[IO.Path]::GetFullPath($helper);args=$commandArgs;cwd=(Get-Location).ProviderPath} | ConvertTo-Json -Depth 5 -Compress
  [IO.File]::WriteAllText($requestPath, $request, [Text.UTF8Encoding]::new($false))
  & $helper --codux-wrapper-helper native-launch $requestPath
  $commandExitCode = $LASTEXITCODE
} finally {
  if (Test-Path -LiteralPath $requestPath) { Remove-Item -LiteralPath $requestPath -Force }
}
exit $commandExitCode
