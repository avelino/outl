# Test fixtures

Captured artifacts the test suite decodes verbatim.
They exist because approximating a format the code no longer writes is how a
migration test ends up passing against a file no user ever had.

| file | what it is |
|---|---|
| `legacy-snapshot-schema3.bin` | One `snap-<actor>.bin` written by a schema-3 (bincode 1.3.3) build, captured before the postcard migration (issue #207). Body: one actor, one node under `ROOT`, one property, collapsed, snoozed, block text `"hello legacy"`. Every "old snapshot on disk" test decodes these exact bytes. |

Nothing here is generated at build time — regenerating a fixture defeats its
purpose. If a fixture needs to change, that is a new fixture.
