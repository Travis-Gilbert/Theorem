# rustyred-thg-behavior-ir

Portable behavior IR contracts for feature-port reconstruction.

This crate is intentionally substrate-light. Source-specific frontends lower
evidence into feature slices and behavior contracts; target emitters interpret
those contracts into target plans, patch sets, and validation receipts.
