![cargo test](https://github.com/curatorsigma/churchtools-ta-sync/actions/workflows/cargo-test.yml/badge.svg)

# What is this repo?
`ct-ta-sync` reads room bookings from churchtools and forwards that information to a CMI.
This information may be further used to turn on/off heating etc.

# Getting started
## Prepare the config:
You may copy the `config.example.yaml` to `/etc/ct-ta-sync/config.yaml` and then edit this file.

## Setup the container
```bash
docker compose up
```

## Setup the integration in your CMI
- Optional: Send the current external temperature to the Host running the sync. This allows us to scale preheating and preshutdown times to be more energy efficient.
- Use the room data. It is sent as a bool (Digital On/Off), and can be used in your programming.

# Further Reading
This project connects to the CMI from [Technische Alternative RT GmbH](https://ta.co.at).
You can find further information on [their wiki](https://wiki.ta.co.at/Hauptseite).

# You do not want to sync from CT?
You may want to take a look at [coe](https://github.com/curatorsigma/coe-rs). It defines a low-level API to work with COE packets and may be used to
implement any other integration for CMIs.

