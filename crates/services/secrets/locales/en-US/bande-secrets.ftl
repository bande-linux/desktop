password = Password
verify = Verify
cancel = Cancel
ok = OK
deny = Deny
allow = Allow
always-allow = Always Allow
create = Create
not-now = Not Now
update-password = Update Password
new-keychain = Create New Keychain
join = Join
an-app = An app
unlock-title = “{ $app }” wants to use the “{ $keychain }” keychain.
unlock-message = Enter the keychain password.
allow-title = { $count ->
    [one] “{ $app }” wants to use your confidential information stored in “{ $item }” in your keychain.
   *[other] “{ $app }” wants to use { $count } passwords stored in your keychain.
}
allow-message = To allow this, enter the “{ $keychain }” keychain password.
create-title = “{ $app }” wants to create a new keychain, “{ $keychain }”.
create-message = Choose a password for the new keychain.
verify-message = Enter the password again to verify it.
mismatch = The passwords don’t match. Try again.
login-title = Your login keychain was not unlocked with your login password.
login-message = Enter the keychain’s old password to update it to your login password, or create a new keychain. The old keychain is kept.
wrong-password = The password you entered is incorrect.
network-title = The network “{ $network }” requires a password.
network-message = Enter the password to join it.
