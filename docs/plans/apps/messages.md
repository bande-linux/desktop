# Messages

Domain: [messages](../../architecture/intents/domains/messages.md). Package A8, after A1b. Size XL.

**There is.** Service discovery, HTTPS, keyring, notifications, rich text. `xmpp-parsers`, `jid` and
`minidom` are packaged; for Matrix only `ruma-common` and `vodozemac` are.

**Decide first.** XMPP or Matrix. Proposal: XMPP (RFC 6120 and 6121), because its parts are
packaged, its servers are many and signing in is a password; Matrix needs a large vendored set and
often a web sign-in.

**Work.** Stream and roster; messages, archive (XEP-0313), uploads (XEP-0363), groups (XEP-0045),
receipts, end-to-end encryption (XEP-0384) with keys checked by the person; a local store and
search; background connection only for accounts the person has.

**Done when** it interoperates with two other clients and two servers, encryption included. Services
without an open protocol are not reached.
