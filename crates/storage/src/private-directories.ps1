$ErrorActionPreference = 'Stop'
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = New-Object System.Security.AccessControl.DirectorySecurity
$acl.SetOwner($sid)
$acl.SetAccessRuleProtection($true, $false)
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow')
$acl.AddAccessRule($rule)
function Assert-Private($entry, $directory) {
    if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'reparse point' }
    $security = $entry.GetAccessControl()
    $rules = $security.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])
    if ($rules.Count -ne 1 -or $rules[0].IdentityReference -ne $sid -or
        $rules[0].AccessControlType -ne 'Allow' -or $rules[0].FileSystemRights -ne 'FullControl') { throw 'unexpected ACL' }
    if ($directory -and ($security.GetOwner([System.Security.Principal.SecurityIdentifier]) -ne $sid -or
        $rules[0].InheritanceFlags -ne $rule.InheritanceFlags -or $rules[0].PropagationFlags -ne 'None')) { throw 'unexpected directory ACL' }
}
$root = $env:ENVLUME_ACL_ROOT
foreach ($name in @('', 'data', 'logs', 'tmp', 'backups')) {
    $path = if ($name) { [IO.Path]::Combine($root, $name) } else { $root }
    $dir = [IO.Directory]::CreateDirectory($path, $acl)
    Assert-Private $dir $true
}
foreach ($suffix in @('', '-wal', '-shm', '-journal')) {
    $file = [IO.FileInfo]::new([IO.Path]::Combine($root, 'data', ('envlume.db' + $suffix)))
    if ($file.Exists) { Assert-Private $file $false }
}
