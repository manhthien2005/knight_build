# Script khởi động nhanh Zeus UI trên Desktop người dùng
Stop-Process -Name zeus-ui -Force -ErrorAction SilentlyContinue
Remove-Item "D:\Gaming\KnightOnline_402\ZeusPlay\data\.core-instance.lock" -Force -ErrorAction SilentlyContinue
Start-Process -FilePath "D:\Gaming\KnightOnline_402\ZeusPlay\zeus-ui.exe" -WorkingDirectory "D:\Gaming\KnightOnline_402\ZeusPlay"
Write-Host "Da khoi dong Zeus UI thanh cong!"
