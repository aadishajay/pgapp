# Vendored third-party assets

These files back the App Builder's SQL Workshop editor (SQL Commands +
Object Browser). Both projects are MIT-licensed, compatible with pgapp's
own MIT license (see `../LICENSE`). Fetched from jsDelivr's npm mirror,
vendored flat here (no bundler/npm in this repo) rather than loaded from
a CDN at runtime.

- **CodeMirror 5.65.16** — <https://codemirror.net/5/> —
  Copyright (C) 2017 by Marijn Haverbeke and others — MIT License
  - `codemirror.min.js`, `codemirror.min.css`
  - `codemirror-sql.min.js` (`mode/sql/sql.js`)
  - `codemirror-show-hint.min.js`, `codemirror-show-hint.min.css` (`addon/hint/show-hint.js`)
  - `codemirror-sql-hint.min.js` (`addon/hint/sql-hint.js`)
  - `codemirror-matchbrackets.min.js` (`addon/edit/matchbrackets.js`)

- **sql-formatter 15** — <https://github.com/sql-formatter-org/sql-formatter> —
  MIT License
  - `sql-formatter.min.js`
