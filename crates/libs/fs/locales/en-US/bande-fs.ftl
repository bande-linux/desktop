untitled-folder = untitled folder
copy-name = { $name } copy
copy-name-numbered = { $name } copy { $number }
alias-name = { $name } alias
alias-name-numbered = { $name } alias { $number }

size-bytes = { $count ->
    [0] Zero bytes
    [one] { $count } byte
   *[other] { $count } bytes
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB
archive-name = Archive
date-today = Today, { $time }
date-yesterday = Yesterday, { $time }
