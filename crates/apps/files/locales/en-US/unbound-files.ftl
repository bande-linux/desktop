folder = Folder
document = Document
item-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
}

size-bytes = { $count ->
    [0] Zero bytes
    [one] { $count } byte
   *[other] { $count } bytes
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB
