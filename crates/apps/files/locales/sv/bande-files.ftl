folder = Mapp
trash = Papperskorg
sidebar-favourites = Favoriter
sidebar-locations = Platser
empty-folder = Tom mapp
trash-empty = Papperskorgen är tom
recents-empty = Filer du öppnar visas här
no-results = Inga resultat för ”{ $text }” i { $folder }
search-home = Sök i hemmappen
no-permission = Du har inte behörighet att se innehållet i ”{ $folder }”.
no-selection = Inget markerat
column-name = Namn
search-placeholder = Sök
column-modified = Ändringsdatum
column-created = Skapad
column-size = Storlek
column-kind = Sort
column-move-left = Flytta åt vänster
column-move-right = Flytta åt höger
menu-columns = Kolumner
menu-move-column-left = Flytta { $column } åt vänster
menu-move-column-right = Flytta { $column } åt höger
item-count = { $count ->
    [one] { $count } objekt
   *[other] { $count } objekt
}
selected-count = { $selected } av { $count } markerade
status-free = { $items }, { $free } tillgängligt

info-name = Namn och filändelse
info-size = Storlek
info-modified = Ändrad
info-created = Skapad
more-info = Mer info
kind-and-size = { $kind } · { $size }
info-where = Plats
info-original-location = Läggs tillbaka i
info-deleted = Raderad
info-permissions = Behörigheter
info-link = Pekar på
info-dimensions = Dimensioner
info-pixels = { $width } × { $height }
name-and-extension = Namn och filändelse
hide-extension = Dölj filändelse
sharing-permissions = Delning och behörigheter
allow-running = Tillåt körning som program
access-you = Du
access-group = Grupp
access-everyone = Alla
access-read-write = Läsa och skriva
access-read-only = Endast läsning
access-none = Ingen åtkomst
error-permissions = Det gick inte att ändra behörigheterna: { $error }
folder-size = { $size } för { $count ->
    [one] { $count } objekt
   *[other] { $count } objekt
}
calculating = Beräknar…
open-with = Öppna med
default-app = { $app } (standard)
open = Öppna
use-for-all = Använd för alla

progress-copy = Kopierar { $done } av { $total } objekt
progress-move = Flyttar { $done } av { $total } objekt
progress-duplicate = Duplicerar { $done } av { $total } objekt
progress-trash = Flyttar { $done } av { $total } objekt till papperskorgen
progress-put-back = Lägger tillbaka { $done } av { $total } objekt
progress-rename = Byter namn
progress-new-folder = Skapar en mapp
progress-undo = Ångrar
progress-redo = Gör om
progress-erase = Raderar { $done } av { $total } objekt
progress-bytes = { $done } av { $total }

conflict-title = Ett objekt med namnet ”{ $name }” finns redan på den här platsen.
conflict-detail = Befintligt: { $existing }
    Nytt: { $new }
apply-to-all = Använd för alla
stop = Stoppa
skip = Hoppa över
replace = Ersätt
keep-both = Behåll båda
skip-all = Hoppa över alla
try-again = Försök igen
ok = OK
internal-disk = Intern
failure-title = ”{ $name }” kunde inte { $action ->
    [copy] kopieras
    [move] flyttas
    [duplicate] dupliceras
    [trash] flyttas till papperskorgen
    [put-back] läggas tillbaka
    [delete] raderas
   *[other] ändras
}.
failure-permission = Du har inte behörighet.
failure-read-only = Volymen är skrivskyddad.
failure-no-space = Skivan är full.
failure-name-too-long = Namnet är för långt.
failure-name-invalid = Namnet är inte tillåtet på den här skivan.
failure-too-large = Objektet är för stort för den här skivans format.
failure-device-gone = Skivan kopplades från.
failure-disk-error = Det gick inte att läsa från eller skriva till skivan. Den kan ha kopplats från eller vara skadad.
failure-missing = Objektet eller dess mapp finns inte längre.
no-trash-title = ”{ $name }” kan inte flyttas till papperskorgen.
no-trash-detail = Den här skivan har ingen papperskorg som går att använda. Vill du radera objektet nu? Det går inte att ångra.
delete-all = Radera alla
busy-title = Filer arbetar fortfarande.
busy-detail = Objekt som redan är klara blir kvar där de är. Objektet som pågår kasseras, och originalet rörs inte.
stop-and-quit = Stoppa och avsluta
stop-and-close = Stoppa och stäng
keep-running = Fortsätt
failure-other = { $error }
no-space-item = ”{ $name }” kan inte { $action ->
    [move] flyttas
    [duplicate] dupliceras
   *[other] kopieras
} eftersom ”{ $volume }” behöver { $size } mer utrymme.
no-space-items = Objekten kan inte { $action ->
    [move] flyttas
    [duplicate] dupliceras
   *[other] kopieras
} eftersom ”{ $volume }” behöver { $size } mer utrymme.
no-space-detail = Ingenting skrevs. Frigör utrymme på skivan och försök igen.

cancel = Avbryt
empty-trash = Töm papperskorgen
empty-trash-title = Vill du verkligen radera objekten i papperskorgen permanent?
delete = Radera
delete-title = { $count ->
    [one] Vill du radera det här objektet direkt?
   *[other] Vill du radera de här { $count } objekten direkt?
}
cannot-undo = Det går inte att ångra den här åtgärden.

rename-empty = Ett namn får inte vara tomt.
rename-slash = Ett namn får inte innehålla ”/”.
rename-reserved = Namnen ”.” och ”..” är reserverade av systemet.
rename-exists = Namnet ”{ $name }” är redan upptaget. Välj ett annat namn.
error-operation = Det gick inte att slutföra åtgärden: { $error }
error-item = Det gick inte att hantera ”{ $name }”: { $error }{ $others ->
    [0] {""}
   *[other] {" "}({ $others } till)
}
go-to-title = Gå till mappen:
go = Gå
error-go-to = ”{ $path }” hittades inte.
error-default-app = Det gick inte att ändra standardappen: { $error }

menu-open-with = Öppna med
menu-open-with-other = Annan…
open-with-title = Välj en app för att öppna ”{ $name }”:
open-with-items-title = Välj en app för att öppna { $count } objekt:
always-open-with = Öppna alltid med
menu-quick-look = Snabbtitt
menu-quick-look-full-screen = Snabbtitt i helskärm
menu-get-info = Visa info
menu-show-in-enclosing-folder = Visa i överordnad mapp
menu-rename = Byt namn
menu-rename-items = Byt namn på { $count } objekt…
batch-title = Byt namn på { $count } objekt
batch-replace-text = Ersätt text
batch-add-text = Lägg till text
batch-format-names = Format
batch-find = Sök efter:
batch-replace-with = Ersätt med:
batch-text = Text:
batch-where = Plats:
batch-after-name = efter namnet
batch-before-name = före namnet
batch-format = Format:
batch-name-index = Namn och index
batch-name-counter = Namn och räknare
batch-custom-format = Eget format:
batch-start-at = Börja numrera på:
batch-start-number = Börja numrera på ett heltal.
batch-twice = Två objekt skulle båda heta ”{ $name }”.
batch-example = { $old } → { $new }
menu-duplicate = Duplicera
menu-make-alias = Skapa alias
menu-copy = Kopiera
menu-copy-path = Kopiera som sökväg
menu-copy-path-of = Kopiera ”{ $name }” som sökväg
menu-copy-paths = Kopiera { $count } objekt som sökvägar
menu-add-to-sidebar = Lägg till i sidofältet
menu-undo-action = Ångra { $action }
menu-redo-action = Gör om { $action }
edit-copy = kopiering av { $count ->
    [one] { $count } objekt
   *[other] { $count } objekt
}
edit-move = flytt av { $count ->
    [one] { $count } objekt
   *[other] { $count } objekt
}
edit-duplicate = duplicering av { $count ->
    [one] { $count } objekt
   *[other] { $count } objekt
}
edit-rename = namnbyte
edit-new-folder = ny mapp
edit-new-document = nytt dokument
edit-trash = flytt till papperskorgen
edit-put-back = återläggning
edit-compress = komprimering
edit-extract = uppackning
edit-make-alias = skapande av alias
menu-cut = Klipp ut
menu-move-to-trash = Flytta till papperskorgen
menu-move-items-to-trash = Flytta { $count } objekt till papperskorgen
menu-put-back = Lägg tillbaka
menu-delete-immediately = Radera direkt…
menu-new-folder = Ny mapp
menu-paste = Klistra in
menu-sort-by = Sortera efter
menu-use-groups = Använd grupper
menu-group-by = Gruppera efter
menu-group-none = Ingen
menu-group-application = Program
menu-group-added = Datum tillagd
group-show-all = Visa alla
group-show-less = Visa färre
menu-as-icons = som ikoner
menu-as-columns = som kolumner
menu-as-gallery = som galleri
menu-as-list = som lista
menu-bigger-icons = Större ikoner
menu-smaller-icons = Mindre ikoner
menu-show-hidden-files = Visa dolda filer
menu-hide-hidden-files = Dölj dolda filer
menu-empty-trash = Töm papperskorgen…
menu-remove-from-sidebar = Ta bort från sidofältet
menu-eject = Mata ut
eject-busy = Skivan ”{ $name }” matades inte ut eftersom ett eller flera program kan använda den.
eject-busy-detail = Klicka på Tvinga utmatning för att mata ut skivan direkt. Program som använder den kan förlora ändringar som inte har sparats.
eject-failed = Det gick inte att mata ut skivan ”{ $name }”.
force-eject = Tvinga utmatning

app-name = Filer
menu-file = Arkiv
menu-edit = Redigera
menu-view = Visning
menu-go = Gå
menu-open = Öppna
menu-find = Sök
menu-undo = Ångra
menu-redo = Gör om
menu-move-item-here = Flytta objektet hit
menu-select-all = Markera allt
menu-back = Bakåt
menu-forward = Framåt
menu-enclosing-folder = Överordnad mapp
menu-desktop = Skrivbord
menu-documents = Dokument
menu-downloads = Hämtningar
menu-home = Hem
menu-go-to-folder = Gå till mapp…
menu-recent-folders = Senaste mappar
menu-clear-menu = Rensa menyn
icon-size = Ikonstorlek
menu-open-in-new-tab = Öppna i ny flik
menu-new-window = Nytt fönster
menu-new-tab = Ny flik
menu-previous-tab = Visa föregående flik
menu-next-tab = Visa nästa flik
recents = Senaste
search-title = Söker i ”{ $folder }”
search-scope = Sök:
search-scope-folder = ”{ $folder }”
search-scope-home = Hem
search-kind = Sort
search-date = Ändringsdatum
kind-any = Alla sorter
kind-folder = Mapp
kind-alias = Alias
kind-document = Dokument
kind-image = Bild
kind-audio = Ljud
kind-video = Video
kind-archive = Arkiv
date-any-time = När som helst
date-today-filter = Idag
date-last-week = Senaste 7 dagarna
date-last-month = Senaste 30 dagarna
date-this-year = I år
column-where = Plats
view-label = Visning
view-icons = Ikoner
view-list = Lista
view-columns = Kolumner
view-gallery = Galleri
menu-compress = Komprimera ”{ $name }”
menu-compress-items = Komprimera { $count } objekt
progress-compress = Komprimerar { $done } av { $total } objekt
progress-extract = Packar upp { $done } av { $total } objekt
menu-compress-selection = Komprimera
menu-open-in-terminal = Öppna i Terminal
menu-new-document = Nytt dokument
menu-status-bar = Visa eller dölj statusfältet
menu-restore-sidebar = Återställ standardsidofältet
settings-title = Inställningar för Filer
settings-general = Allmänt
settings-new-window = Nya fönster visar
settings-home = Hem
settings-current-folder = Aktuell mapp
settings-last-folder = Senaste mappen
settings-other-folder = Annan…
settings-choose-new-window-folder = Välj mappen som nya fönster visar
settings-default-view = Standardvisning
settings-show-hidden = Visa dolda filer
settings-desktop-disks = Visa externa skivor på skrivbordet
settings-spring-loading = Mappar som öppnas vid hovring
settings-spring-delay = Fördröjning för att öppna mappar vid hovring
settings-folders-first = Behåll mappar överst vid sortering efter namn
settings-sidebar = Visa i sidofältet
settings-external-disks = Externa skivor
settings-advanced = Avancerat
settings-show-extensions = Visa alla filändelser
settings-warn-extension-change = Varna innan en filändelse ändras
settings-warn-empty-trash = Varna innan papperskorgen töms
settings-purge-trash = Ta bort objekt från papperskorgen efter 30 dagar
settings-search-in = Sök i

menu-browse-backups = Bläddra bland säkerhetskopior…
menu-restore-from-backup = Återställ
backups-from = Säkerhetskopia från { $when }
backups-older = Äldre
backups-newer = Nyare
backups-restore = Återställ
backups-done = Klar
backups-not-set-up = Säkerhetskopiering är inte inställd. Välj en skiva i Inställningar ▸ Säkerhetskopia.
backups-disk-away = Anslut säkerhetskopieringsskivan för att bläddra bland säkerhetskopior.
backups-cannot-open = Det gick inte att öppna säkerhetskopiorna.
backups-none = Det finns ännu ingen säkerhetskopia av den här mappen.
backups-no-older = Ingen äldre version.
backups-no-newer = Ingen nyare version.
backups-cannot-restore = Det gick inte att återskapa originalmappen.
menu-connect-to-server = Anslut till server…
server-connect-title = Anslut till server
server-address-hint = smb://server/share, nfs://server/export, sftp://server eller https://server/dav
server-recent = Senaste servrar
server-reconnect = Anslut igen vid inloggning
server-connect = Anslut
server-connecting = Ansluter till { $server }…
server-installing = Installerar det som behövs…
server-unreachable = Det går inte att nå servern.
server-unreachable-detail = Kontrollera adressen och att du är på samma nätverk.
server-no-share = Servern har ingen sådan delad mapp.
server-no-shares = Servern delar inga mappar med dig.
server-invalid = Det är inte en serveradress.
server-failed = Anslutningen misslyckades.
server-user = Namn
server-password = Lösenord
server-remember = Kom ihåg det här lösenordet i nyckelringen
server-password-title = Logga in på { $server }
server-password-refused = Namnet eller lösenordet godkändes inte.
server-choose-share = Välj en delad mapp på { $server }
server-new-key = Den här datorn har inte anslutit till { $server } tidigare.
server-new-key-detail = Anslut bara om du litar på servern. Dess fingeravtryck är { $key }.
server-key-changed = Serverns identitet har ändrats.
server-key-changed-detail = Det kan vara ett angrepp, eller så har servern installerats om. Fråga den som driver den och ta bort dess gamla nyckel från ~/.ssh/known_hosts för att ansluta.
server-needs-install = För att ansluta till den här typen av server krävs programvara som inte är installerad.
server-needs-install-detail = Det är en liten hämtning från Bandes programarkiv.
server-install = Installera
server-waiting = Väntar på servern…
server-not-responding = Servern svarar inte. Filer här går inte att komma åt just nu.
server-disconnect = Koppla från
sidebar-network = Nätverk
menu-share = Dela
menu-share-mail = E-post
menu-share-to = Kopiera till { $place }
failure-server-gone = Servern slutade svara. Ingenting lämnades kvar under det riktiga namnet. Försök igen när den är tillbaka.
