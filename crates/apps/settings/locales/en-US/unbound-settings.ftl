title = System Settings
menu-view = View
search = Search
not-changed = Not changed: { $error }

wifi = Wi-Fi
wifi-keywords = wireless, wlan, internet, network
wifi-power-keywords = on, off, airplane
networks = Networks
networks-keywords = join, connect, ssid, hotspot, password
searching-networks = Looking for networks…
connected = Connected
not-connected = Not connected
joining = Joining…
saved = Saved
password = Password
password-for = Password for “{ $network }”
password-hint = Press Return to join or Escape to cancel
wrong-password = The password is not correct. Try again.
join-failed = Could not join the network ({ $reason }).

bluetooth = Bluetooth
bluetooth-keywords = wireless, headphones, mouse, keyboard, speaker
bluetooth-power-keywords = on, off
my-devices = My devices
my-devices-keywords = paired, connect, disconnect, headphones
nearby-devices = Nearby devices
nearby-devices-keywords = pair, add, discover, new device
no-devices = No devices paired yet
searching-devices = Looking for devices…
connect = Connect
disconnect = Disconnect
connecting = Connecting…
pair = Pair
pairing = Pairing…
pairing-request = Pairing request
confirm-code = Does { $device } show the code { $code }?
authorize-device = Allow { $device } to connect?
type-code = Type { $code } on { $device }, then press Return on it.
enter-pin = Enter the PIN for { $device } and press Return.
pin = PIN
cancel = Cancel
deny = Deny
allow = Allow

notifications = Notifications
notifications-keywords = alerts, banners
do-not-disturb = Do Not Disturb
do-not-disturb-keywords = focus, quiet, silence, mute notifications, banners

sound = Sound
sound-keywords = audio, speakers, headphones, microphone
output = Output
output-volume = Output volume
output-volume-keywords = loudness, speakers, level
mute = Mute
mute-keywords = silence, sound off
output-device = Play sound through
output-device-keywords = speakers, headphones, output device
input = Input
input-volume = Input volume
input-volume-keywords = microphone, gain, recording, level
mute-input = Mute microphone
mute-input-keywords = silence, microphone off
input-device = Record sound from
input-device-keywords = microphone, input device, recording
no-sound-devices = No devices

displays = Displays
displays-keywords = monitor, screen, resolution
display-scale = Size of text and controls
display-scale-keywords = scale, zoom, resolution, hidpi, retina, bigger, smaller
scale-automatic = Auto
percent = { $value }%
refresh-rate = Refresh rate
refresh-rate-keywords = hertz, hz, smooth, frame rate
hertz = { $value } Hz
variable-refresh-rate = Variable refresh rate
variable-refresh-rate-keywords = vrr, adaptive sync, freesync, promotion, smooth

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
