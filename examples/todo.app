# Example application definition in pgapp's markup language.
# This one file is the single source of truth: it is parsed once at
# startup and synced into the in-database metadata (pgapp_meta.*),
# which is what actually drives routing and rendering afterwards.
#
# Named-query SQL below references pgapp_data.todo_tasks directly —
# that's this entity's physical table name (<app slug>_<entity slug>),
# printed at startup. A named query isn't translated from the logical
# entity name, so it has to know the real table.

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
    item "Open" -> page OpenTasks
    item "More" {
      item "About" -> page About
    }
  }

  # App-scoped: visible from every page's LOVs/regions.
  query assignees {
    sql: "select distinct assignee as value from pgapp_data.todo_tasks where assignee is not null order by 1"
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
  # (text, checkbox); priority/assignee/notes declare one explicitly,
  # with assignee's popup sourced from the app-scoped query above
  # instead of a fixed list.
  page "Tasks" as list of tasks {
    columns: title, priority, done, created_at
    form: title, priority, done, assignee, notes
    link: title -> page TaskDetail (priority: priority)
    item priority as radio ("Low", "Medium", "High")
    item assignee as popup from query assignees
    item notes as readonly

    # Page-scoped: only this page's items/LOVs can see "recent".
    query recent {
      sql: "select id, title, priority, done from pgapp_data.todo_tasks order by id desc limit 5"
    }
    items {
      text "Manage your tasks below. Click a title to see its detail page."
      region "Recently added" from query recent
    }
  }

  page "TaskDetail" as detail of tasks {
    # :priority binds from the current row by default, or from the
    # ?priority= the Tasks list forwarded via its link: (...) — either
    # way it's the same cross-page-parameter mechanism.
    query siblings {
      sql: "select id, title from pgapp_data.todo_tasks
            where priority = :priority::text and id != :id::integer
            order by id"
    }
    items {
      region "Other tasks with the same priority" from query siblings
    }
  }

  # A list page whose rows come from a named query instead of a flat
  # `select * from` the entity table — create/edit/delete still write
  # to the entity by id regardless.
  page "OpenTasks" as list of tasks {
    query open {
      sql: "select id, title, priority, assignee from pgapp_data.todo_tasks
            where done = false order by id"
    }
    source: query open
    columns: title, priority, assignee
    form: title, priority, done, assignee, notes
  }

  page "About" as static {
    items {
      text "pgapp is an Oracle APEX-inspired no/low-code framework built on Postgres."
      link "Back to tasks" -> page Tasks
    }
  }
}
