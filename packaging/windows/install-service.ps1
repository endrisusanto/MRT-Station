param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,
    [string]$InventoryPath = "$env:ProgramData\EM Station\devices.json"
)

$ErrorActionPreference = "Stop"
$serviceName = "EMAgent"
$existing = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($existing) {
    Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
    sc.exe delete $serviceName | Out-Null
}

sc.exe create $serviceName binPath= "`"$BinaryPath`"" start= auto DisplayName= "EM Station Agent" | Out-Null
sc.exe description $serviceName "Local device and token service for EM Station" | Out-Null
sc.exe failure $serviceName reset= 86400 actions= restart/2000/restart/5000/none/0 | Out-Null
[Environment]::SetEnvironmentVariable("EM_DEVICE_INVENTORY", $InventoryPath, "Machine")
Start-Service -Name $serviceName
