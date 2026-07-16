# Example application definition in pgapp's markup language.
# This one file is the single source of truth: it is parsed once at
# startup and synced into the in-database metadata (pgapp_meta.*),
# which is what actually drives routing and rendering afterwards.

app "Todo" {
  nav {
    item "Tasks" -> page Tasks
    item "More" {
      item "About" -> page About
    }
  }

  entity "tasks" {
    field id: id
    field title: text required
    field done: boolean default false
    field created_at: timestamp default now
  }

  page "Tasks" as list of tasks {
    columns: title, done, created_at
    form: title, done
    link: title -> page TaskDetail
    items {
      text "Manage your tasks below. Click a title to see its detail page."
    }
  }

  page "TaskDetail" as detail of tasks {
  }

  page "About" as static {
    items {
      text "pgapp is an Oracle APEX-inspired no/low-code framework built on Postgres."
      link "Back to tasks" -> page Tasks
    }
  }
}
