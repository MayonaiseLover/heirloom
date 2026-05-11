# Heirloom v0.1 demo tape
# Run with: vhs assets/demo.tape
# Produces: assets/demo.gif

Output assets/demo.gif

Set FontSize 18
Set Width 1100
Set Height 720
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms
Set Padding 30
Set Shell bash
Set WindowBar Colorful

Type "heirloom init" Sleep 200ms Enter
Sleep 1500ms

Type "heirloom ingest fs --path ~/notes" Sleep 200ms Enter
Sleep 1800ms

Type "heirloom search 'the auth bug Sam was reviewing'" Sleep 200ms Enter
Sleep 2500ms

Type "# now from Claude Desktop, just ask..." Sleep 200ms Enter
Sleep 1500ms

Type "heirloom serve" Sleep 200ms Enter
Sleep 2500ms
