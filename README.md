![cargo test](https://github.com/curatorsigma/churchtools-ta-sync/actions/workflows/cargo-test.yml/badge.svg)

# What is this repo?
`ct-ta-sync` reads room bookings from churchtools and forwards that information to a CMI.
This information may be further used to turn on/off heating etc.

# Getting started
## Get your CT Login token
TODO
## Setup the container
TODO: docker compose up
## Setup the integration in your CMI
- output external temperature
- go ahead and use the room data

# Further Reading
This project connects to the CMI from [Technische Alternative RT GmbH](https://ta.co.at).
You can find further information on [their wiki](https://wiki.ta.co.at/Hauptseite).

# You do not want to sync from CT?
You may want to take a look at `coe`(TODO: link). It defines a low-level API to work with COE packets and may be used to
implement any other integration for CMIs.

