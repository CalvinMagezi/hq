# A reproducible terminal clip. Render with: vhs scripts/media/tapes/install-mac.tape
# It types the Mac install lines and shows no real output, so nothing machine specific is recorded.
Output install-mac.mp4
Set FontSize 22
Set Width 1280
Set Height 720
Set Theme "Dark+"
Type "brew install minisign"
Sleep 500ms
Enter
Sleep 2s
Type "curl -fsSL https://agent-hq.online/install.sh | bash -s -- --pair"
Sleep 800ms
Enter
Sleep 3s
