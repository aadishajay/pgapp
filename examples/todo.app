# Example application definition in pgapp's markup language.
# This one file is the single source of truth: it is parsed once at
# startup and synced into the in-database metadata (pgapp_meta.*),
# which is what actually drives routing and rendering afterwards.

app "Todo" {
  entity "tasks" {
    field id: id
    field title: text required
    field done: boolean default false
    field created_at: timestamp default now
  }

  page "Tasks" as list of tasks {
    columns: title, done, created_at
    form: title, done
  }
}
