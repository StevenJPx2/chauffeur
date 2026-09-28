# shop

A tiny order service. All state lives in one JSON file.

```sh
python3 -m shop.cli --store shop.json add-product MUG "Coffee mug" 12.50
python3 -m shop.cli --store shop.json checkout --customer alice --item MUG:2
python3 -m shop.cli --store shop.json orders --customer alice
```

Money is handled as `Decimal` and rounded to whole cents with halves rounding up (`pricing.money`).

Run the tests with `python3 -m unittest`.
