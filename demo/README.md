# The sample cluster

A made-up freight forwarder, running on a local Temporal dev server. It gives a namespace
the shape a real one has, so tmprl can be shown and tested without anyone's production:

- two workflow types per version, `shipment-v3` and `task_shipment-v3`, and a `v2` of each
  still live beside them
- about 25 activity types in a booking, with a framework's publish and store steps between
  them, a side effect, timers, an update and a signal
- a second workflow that waits on a person, started as a child and answered by an update
- activities that fail for good, that time out, that succeed on the fourth try, and that
  retry for ever
- payloads the cluster cannot read without the sample's codec server

Every name, address, phone number and company in the data is invented.

## Run it

Needs Go and the `temporal` CLI.

```sh
demo/run.sh up          # dev server on 127.0.0.1:7244, codec server, workers
demo/run.sh seed 300    # 300 shipments, the same ones every time
demo/run.sh tmprl       # tmprl against it, with the config in demo/config
demo/run.sh down        # stop it all; the data is in memory and goes with it
```

`seed` takes the seeder's flags after the count: `-seed 7` for a different mix, `-prefix b`
to seed a second batch without colliding with the first, `-rate 100` to go faster.

## What a seeded cluster holds

| Fate | Share | What it looks like |
|---|---|---|
| books and completes | 55% | a full history, six stages |
| waits on an inspector who never answers | 20% | running, parked on a child workflow |
| customs gateway times out three times | 8% | `file_customs_declaration ×4`, then completes |
| carrier has no capacity | 6% | `book_carrier` retrying for ever: the `retrying` panel |
| postcode does not exist | 6% | failed, `ValidationError`, not retried |
| lane rating is too slow | 5% | failed on an activity timeout |

A few are terminated or cancelled by hand afterwards, as someone on call would.

## Going to the source

`demo/config/config.toml` points `gf` at `demo/find-source.sh`, a resolver of a dozen
lines: it reads the name under the cursor from the JSON tmprl sends and prints where the
sample defines it. Press `gf` on `book_carrier` and the editor opens on `func BookCarrier`.

## Recording the README's demo

```sh
demo/run.sh up
for w in 1 2 3 4 5 6; do demo/run.sh seed $((40 + w * 25)) -seed $w -prefix "s$w"; sleep 50; done
vhs demo/demo.tape      # writes docs/img/demo.gif
```

The waves a minute apart are what give the per-minute charts a shape.
