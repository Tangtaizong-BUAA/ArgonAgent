# CSV Quality Small Fixture

Expected profiler behavior:

- Count 5 data rows and 5 columns.
- Detect missing values in `email` and `value`.
- Detect 1 duplicate row.
- Classify `subject_id` and `email` as sensitive personal columns.
- Detect a likely outlier in `value`.

Run:

```bash
python3 scripts/prototype_csv_profiler.py eval/fixtures/research/csv-quality-small/input.csv
```

