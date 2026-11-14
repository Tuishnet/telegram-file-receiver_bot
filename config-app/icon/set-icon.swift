// Назначает файлам пользовательскую иконку: swift set-icon.swift <картинка> <файл>...
import AppKit
let img = NSImage(contentsOfFile: CommandLine.arguments[1])!
for path in CommandLine.arguments.dropFirst(2) {
    print(path, NSWorkspace.shared.setIcon(img, forFile: path, options: []))
}
