# Fingerprint and security keys

Package A11, after C3 and D19. Size M.

**There is.** Sign-in and the lock screen relay the whole PAM conversation, so they use a
fingerprint where PAM is set up for it. The polkit agent hides a message that comes before a prompt.
Nothing enrols one, and the polkit helper cannot see a security key's device.

**Work.** Settings ▸ Password and Sign-In: enrol, name and remove fingers (`fprintd`) after the
password; turning PAM's part on the way Debian does (`pam-auth-update`), never by overriding files;
security keys (`pam_u2f`) at sign-in and the lock screen; at the lock screen password and finger at
once, the first to finish wins; `fprintd` and `libpam-u2f` as `Suggests`.

**Done when** the prompt, sign-in, lock and `sudo` accept the same finger, the password always
works, and removing the packages breaks nothing.
