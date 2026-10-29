title = System Settings
search = Search
not-changed = Not changed: { $error }

appearance = Appearance
appearance-keywords = theme, dark mode, light mode, colour, color
appearance-light = Light
appearance-dark = Dark
appearance-automatic = Auto

desktop-and-dock = Desktop & Dock
dock-position = Position on screen
dock-position-keywords = dock, panel, taskbar, edge
edge-left = Left
edge-bottom = Bottom
edge-right = Right
edge-top = Top

lock-screen = Lock Screen
dim-after = Dim the display when inactive
dim-after-keywords = idle, brightness, screen
lock-after = Turn display off and lock when inactive
lock-after-keywords = idle, password, screen saver, security
sleep-after = Sleep when inactive
sleep-after-keywords = idle, suspend, energy, battery, power

never = Never
seconds = { $count ->
    [one] { $count } second
   *[other] { $count } seconds
}
minutes = { $count ->
    [one] { $count } minute
   *[other] { $count } minutes
}
hours = { $count ->
    [one] { $count } hour
   *[other] { $count } hours
}
