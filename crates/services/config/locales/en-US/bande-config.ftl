has-error = { $file } has an error
has-errors = { $file } has { $count } errors
error-at = Line { $line }, column { $column }: { $message }.
more-errors = { $count ->
    [one] One more line has an error.
   *[other] { $count } more lines have errors.
}
cannot-read = { $file } cannot be read
read-error = { $error }.
previous-settings-stay = { $detail } The previous settings stay in effect.
other-settings-apply = { $detail } Every other setting is in effect.
open-in-editor = Open in Editor
