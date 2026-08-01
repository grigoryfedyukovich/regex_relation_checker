# Small route-policy evaluation

This corpus models a common maintenance task: checking whether a revised route-validation regex admits new values, loses old values, or collides with another route.

| Check | Left | Right | Expected |
|---|---|---|---|
| slug compatibility | `[a-z0-9-]+` | `[a-z][a-z0-9-]*` | left is **not** included; witness `"-"` |
| version aliases | `v[0-9]+` | `v\d+` | equivalent in v0.1 |
| admin/public split | `admin/.*` | `public/.*` | disjoint in ASCII mode |
| asset overlap | `assets/.*\.css` | `.*\.css` | overlap; a shortest witness exists |
| optional suffix | `item(s)?` | `items?` | equivalent |
| numeric ID tightening | `[0-9]+` | `[1-9][0-9]*` | left is not included; witness `"0"` |

Run the corpus manually:

```bash
regexrel includes '[a-z0-9-]+' '[a-z][a-z0-9-]*'
regexrel equivalent 'v[0-9]+' 'v\d+'
regexrel overlap 'admin/.*' 'public/.*'
regexrel overlap 'assets/.*\.css' '.*\.css'
regexrel equivalent 'item(s)?' 'items?'
regexrel includes '[0-9]+' '[1-9][0-9]*'
```

The evaluation is deliberately small. Its purpose is to demonstrate useful diagnostics and expose semantic assumptions, not to claim broad regex-engine compatibility.
