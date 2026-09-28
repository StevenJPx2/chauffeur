# spending-report

Turns the bank's monthly CSV export into a per-category spending summary.

```sh
node src/index.js data/transactions.csv config/report.json
npm test   # or: node test/run.js
```

## Input

`data/transactions.csv` is the export straight from the bank: a header row
(`date,description,category,amount`) followed by one transaction per row. It is
standard CSV, so fields may be wrapped in double quotes; the bank quotes any
field containing a comma, and amounts of 1,000 or more are written with
thousands separators (e.g. `"1,500.00"`). Refunds are negative amounts.

## Config

`config/report.json`:

- `title`: first line of the report.
- `budgets`: monthly budget per category. Categories without a budget show `-`
  in the Budget column.
- `exclude`: categories left out of the report entirely (e.g. transfers between
  our own accounts).
