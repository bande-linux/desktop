title = System Settings
search = Search
not-changed = Not changed: { $error }

appearance = Appearance
appearance-keywords = theme, look, colour, color
appearance-mode = Appearance
appearance-mode-keywords = dark mode, light mode, night, theme
appearance-light = Light
appearance-dark = Dark
appearance-automatic = Auto

desktop-and-dock = Desktop & Dock
desktop-and-dock-keywords = dock, desktop
dock = Dock
dock-position = Position on screen
dock-position-keywords = panel, taskbar, edge, left, right, bottom, top
edge-left = Left
edge-bottom = Bottom
edge-right = Right
edge-top = Top

lock-screen = Lock Screen
lock-screen-keywords = security, idle, screen saver, energy
when-inactive = When inactive
dim-after = Dim the display after
dim-after-keywords = idle, brightness, screen
lock-after = Turn off the display and lock after
lock-after-keywords = idle, password, screen saver, security
sleep-after = Sleep after
sleep-after-keywords = idle, suspend, energy, battery, power
with-display = With display

keyboard = Keyboard
keyboard-keywords = keys, typing
key-repeat = Key repeat
repeat-rate = Repeat rate
repeat-rate-keywords = key repeat, speed, typing, hold
repeat-delay = Delay until repeat
repeat-delay-keywords = key repeat, hold, wait, typing
slow = Slow
fast = Fast
long = Long
short = Short

never = Never
seconds = { $count } s
minutes = { $count } min
hours = { $count ->
    [one] { $count } hour
   *[other] { $count } hours
}
