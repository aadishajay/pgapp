# Example application definition in pgapp's markup language.
# This one file is the single source of truth: it is parsed once at
# startup and synced into the in-database metadata (pgapp_meta.*),
# which is what actually drives routing and rendering afterwards.

app "Todo" {
  header {
    text "pgapp Todo Demo"
  }

  footer {
    text "Built with pgapp - a Postgres-native no/low-code framework."
    link "About" -> page About
  }

  nav {
    item "Tasks" -> page Tasks
    item "More" {
      item "About" -> page About
    }
  }

  entity "tasks" {
    field id: id
    field title: text required
    field priority: text default Medium
    field done: boolean default false
    field assignee: text
    field notes: text
    field created_at: timestamp default now
  }

  # Demonstrates every item type: title/done fall back to their default
  # (text, checkbox); priority/assignee/notes declare one explicitly.
  page "Tasks" as list of tasks {
    columns: title, priority, done, created_at
    form: title, priority, done, assignee, notes
    link: title -> page TaskDetail
    item priority as radio ("Low", "Medium", "High")
    item assignee as popup ("Alice", "Bob", "Carol")
    item notes as readonly
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
