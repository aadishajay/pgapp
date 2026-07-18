-- Big-scale seed for examples/nexus-erp — hundreds of thousands of
-- rows via set-based generate_series (no per-row Python literals,
-- no per-row correlated subqueries for linked fields — those don't
-- scale; this uses one array_agg CTE per linked entity instead).
-- Run against a *fresh* synced database (truncates first).

truncate table pgapp_data.nexus_erp_accounts, pgapp_data.nexus_erp_contacts, pgapp_data.nexus_erp_leads, pgapp_data.nexus_erp_opportunities, pgapp_data.nexus_erp_activities, pgapp_data.nexus_erp_quotes, pgapp_data.nexus_erp_sales_orders, pgapp_data.nexus_erp_invoices, pgapp_data.nexus_erp_payments, pgapp_data.nexus_erp_price_lists, pgapp_data.nexus_erp_products, pgapp_data.nexus_erp_categories, pgapp_data.nexus_erp_warehouses, pgapp_data.nexus_erp_stock_levels, pgapp_data.nexus_erp_stock_movements, pgapp_data.nexus_erp_suppliers, pgapp_data.nexus_erp_purchase_orders, pgapp_data.nexus_erp_purchase_order_lines, pgapp_data.nexus_erp_goods_receipts, pgapp_data.nexus_erp_supplier_invoices, pgapp_data.nexus_erp_employees, pgapp_data.nexus_erp_departments, pgapp_data.nexus_erp_positions, pgapp_data.nexus_erp_leave_requests, pgapp_data.nexus_erp_attendance, pgapp_data.nexus_erp_ledger_accounts, pgapp_data.nexus_erp_expenses, pgapp_data.nexus_erp_budgets, pgapp_data.nexus_erp_ar_invoices, pgapp_data.nexus_erp_ap_bills, pgapp_data.nexus_erp_projects, pgapp_data.nexus_erp_tasks, pgapp_data.nexus_erp_milestones, pgapp_data.nexus_erp_timesheets, pgapp_data.nexus_erp_project_risks, pgapp_data.nexus_erp_tickets, pgapp_data.nexus_erp_agents, pgapp_data.nexus_erp_knowledge_articles, pgapp_data.nexus_erp_slas, pgapp_data.nexus_erp_escalations, pgapp_data.nexus_erp_campaigns, pgapp_data.nexus_erp_marketing_leads, pgapp_data.nexus_erp_email_templates, pgapp_data.nexus_erp_segments, pgapp_data.nexus_erp_events, pgapp_data.nexus_erp_work_orders, pgapp_data.nexus_erp_boms, pgapp_data.nexus_erp_machines, pgapp_data.nexus_erp_maintenance_logs, pgapp_data.nexus_erp_quality_checks, pgapp_data.nexus_erp_assets, pgapp_data.nexus_erp_locations, pgapp_data.nexus_erp_contracts, pgapp_data.nexus_erp_maintenance_requests, pgapp_data.nexus_erp_reservations, pgapp_data.nexus_erp_policies, pgapp_data.nexus_erp_audit_logs, pgapp_data.nexus_erp_approvals, pgapp_data.nexus_erp_notifications, pgapp_data.nexus_erp_documents restart identity cascade;

-- Accounts (30000 rows — hot table)
insert into pgapp_data.nexus_erp_accounts (company_name, status, annual_revenue, is_active, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Account ' || g::text,
  (array['Prospect','Active','Churned'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for accounts.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 30000) g;

-- Leads (2000 rows)
insert into pgapp_data.nexus_erp_leads (lead_name, status, estimated_value, is_hot, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Lead ' || g::text,
  (array['New','Contacted','Converted','Lost'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for leads.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Opportunities (25000 rows — hot table)
insert into pgapp_data.nexus_erp_opportunities (opportunity_name, status, deal_value, is_priority, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Opportunitie ' || g::text,
  (array['Open','Won','Lost'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for opportunities.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 25000) g;

-- Activities (2000 rows)
insert into pgapp_data.nexus_erp_activities (subject, status, duration_minutes, is_billable, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Activitie ' || g::text,
  (array['Planned','Completed','Cancelled'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for activities.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Quotes (2000 rows)
insert into pgapp_data.nexus_erp_quotes (quote_number, status, quote_total, is_expired, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Quote ' || g::text,
  (array['Draft','Sent','Accepted','Rejected'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for quotes.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Invoices (2000 rows)
insert into pgapp_data.nexus_erp_invoices (invoice_number, status, amount_due, is_recurring, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Invoice ' || g::text,
  (array['Unpaid','Paid','Overdue'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for invoices.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Payments (2000 rows)
insert into pgapp_data.nexus_erp_payments (payment_reference, status, payment_amount, is_partial, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Payment ' || g::text,
  (array['Pending','Cleared','Failed'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for payments.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Price lists (2000 rows)
insert into pgapp_data.nexus_erp_price_lists (price_list_name, status, discount_percent, is_default, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Price list ' || g::text,
  (array['Draft','Active','Archived'])[1 + floor(random()*3)::int],
  0 + floor(random()*101)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for price lists.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Products (20000 rows — hot table)
insert into pgapp_data.nexus_erp_products (product_name, status, unit_price, is_taxable, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Product ' || g::text,
  (array['Active','Discontinued','Backordered'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for products.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 20000) g;

-- Categories (2000 rows)
insert into pgapp_data.nexus_erp_categories (category_name, status, sort_order, is_featured, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Categorie ' || g::text,
  (array['Active','Inactive'])[1 + floor(random()*2)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for categories.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Warehouses (2000 rows)
insert into pgapp_data.nexus_erp_warehouses (warehouse_name, status, capacity_units, is_primary, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Warehouse ' || g::text,
  (array['Operational','Maintenance','Closed'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for warehouses.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Stock movements (2000 rows)
insert into pgapp_data.nexus_erp_stock_movements (movement_reference, status, quantity_moved, is_approved, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Stock movement ' || g::text,
  (array['Inbound','Outbound','Transfer','Adjustment'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for stock movements.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Suppliers (2000 rows)
insert into pgapp_data.nexus_erp_suppliers (supplier_name, status, rating_score, is_preferred, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Supplier ' || g::text,
  (array['Approved','Pending','Blacklisted'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for suppliers.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- PO lines (2000 rows)
insert into pgapp_data.nexus_erp_purchase_order_lines (line_reference, status, line_quantity, is_backordered, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' PO line ' || g::text,
  (array['Pending','Received','Cancelled'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for po lines.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Goods receipts (2000 rows)
insert into pgapp_data.nexus_erp_goods_receipts (receipt_number, status, received_quantity, has_discrepancy, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Goods receipt ' || g::text,
  (array['Pending','Inspected','Accepted','Rejected'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for goods receipts.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Supplier invoices (2000 rows)
insert into pgapp_data.nexus_erp_supplier_invoices (invoice_reference, status, invoice_amount, is_overdue, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Supplier invoice ' || g::text,
  (array['Unpaid','Paid','Disputed'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for supplier invoices.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Employees (10000 rows — hot table)
insert into pgapp_data.nexus_erp_employees (full_name, status, annual_salary, is_manager, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Employee ' || g::text,
  (array['Active','OnLeave','Terminated'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for employees.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 10000) g;

-- Departments (2000 rows)
insert into pgapp_data.nexus_erp_departments (department_name, status, headcount, is_cost_center, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Department ' || g::text,
  (array['Active','Restructuring','Closed'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for departments.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Leave requests (2000 rows)
insert into pgapp_data.nexus_erp_leave_requests (request_reference, status, days_requested, is_emergency, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Leave request ' || g::text,
  (array['Pending','Approved','Denied'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for leave requests.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Attendance (2000 rows)
insert into pgapp_data.nexus_erp_attendance (attendance_reference, status, hours_worked, is_overtime, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Attendance ' || g::text,
  (array['Present','Absent','Late','Excused'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for attendance.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Ledger accounts (2000 rows)
insert into pgapp_data.nexus_erp_ledger_accounts (account_name, status, current_balance, is_reconciled, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Ledger account ' || g::text,
  (array['Active','Frozen','Closed'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for ledger accounts.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Expenses (2000 rows)
insert into pgapp_data.nexus_erp_expenses (expense_reference, status, expense_amount, is_billable, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Expense ' || g::text,
  (array['Submitted','Approved','Reimbursed','Rejected'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for expenses.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Budgets (2000 rows)
insert into pgapp_data.nexus_erp_budgets (budget_name, status, budget_amount, is_locked, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Budget ' || g::text,
  (array['Draft','Approved','Exceeded'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for budgets.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- AP bills (2000 rows)
insert into pgapp_data.nexus_erp_ap_bills (bill_number, status, bill_amount, is_recurring, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' AP bill ' || g::text,
  (array['Received','Approved','Paid'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for ap bills.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Projects (8000 rows — hot table)
insert into pgapp_data.nexus_erp_projects (project_name, status, budget_allocated, is_at_risk, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Project ' || g::text,
  (array['Planning','Active','OnHold','Completed'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for projects.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 8000) g;

-- Milestones (2000 rows)
insert into pgapp_data.nexus_erp_milestones (milestone_name, status, completion_percent, is_critical, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Milestone ' || g::text,
  (array['Upcoming','Reached','Missed'])[1 + floor(random()*3)::int],
  0 + floor(random()*101)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for milestones.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Timesheets (2000 rows)
insert into pgapp_data.nexus_erp_timesheets (timesheet_reference, status, hours_logged, is_overtime, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Timesheet ' || g::text,
  (array['Draft','Submitted','Approved'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for timesheets.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Project risks (2000 rows)
insert into pgapp_data.nexus_erp_project_risks (risk_name, status, impact_score, is_high_priority, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Project risk ' || g::text,
  (array['Identified','Mitigating','Resolved','Escalated'])[1 + floor(random()*4)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for project risks.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Tickets (50000 rows — hot table)
insert into pgapp_data.nexus_erp_tickets (subject, status, satisfaction_score, is_urgent, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Ticket ' || g::text,
  (array['Open','InProgress','Resolved','Closed'])[1 + floor(random()*4)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for tickets.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 50000) g;

-- Agents (2000 rows)
insert into pgapp_data.nexus_erp_agents (agent_name, status, tickets_closed, is_lead, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Agent ' || g::text,
  (array['Active','Away','Offline'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for agents.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Knowledge articles (2000 rows)
insert into pgapp_data.nexus_erp_knowledge_articles (article_title, status, view_count, is_featured, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Knowledge article ' || g::text,
  (array['Draft','Published','Archived'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for knowledge articles.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- SLAs (2000 rows)
insert into pgapp_data.nexus_erp_slas (sla_name, status, response_minutes, is_critical, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' SLA ' || g::text,
  (array['Active','Breached','Expired'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for slas.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Campaigns (2000 rows)
insert into pgapp_data.nexus_erp_campaigns (campaign_name, status, budget_spent, is_paid, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Campaign ' || g::text,
  (array['Planned','Running','Completed','Cancelled'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for campaigns.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Email templates (2000 rows)
insert into pgapp_data.nexus_erp_email_templates (template_name, status, open_rate_percent, is_default, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Email template ' || g::text,
  (array['Draft','Active','Retired'])[1 + floor(random()*3)::int],
  0 + floor(random()*101)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for email templates.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Segments (2000 rows)
insert into pgapp_data.nexus_erp_segments (segment_name, status, audience_size, is_dynamic, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Segment ' || g::text,
  (array['Active','Inactive'])[1 + floor(random()*2)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for segments.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Events (2000 rows)
insert into pgapp_data.nexus_erp_events (event_name, status, attendee_count, is_virtual, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Event ' || g::text,
  (array['Planned','Live','Completed','Cancelled'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for events.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Work orders (2000 rows)
insert into pgapp_data.nexus_erp_work_orders (work_order_number, status, estimated_hours, is_priority, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Work order ' || g::text,
  (array['Scheduled','InProgress','Completed','Cancelled'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for work orders.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- BOMs (2000 rows)
insert into pgapp_data.nexus_erp_boms (bom_name, status, component_count, is_active, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' BOM ' || g::text,
  (array['Draft','Approved','Obsolete'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for boms.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Machines (2000 rows)
insert into pgapp_data.nexus_erp_machines (machine_name, status, uptime_percent, needs_service, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Machine ' || g::text,
  (array['Running','Idle','Maintenance','Down'])[1 + floor(random()*4)::int],
  0 + floor(random()*101)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for machines.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Quality checks (2000 rows)
insert into pgapp_data.nexus_erp_quality_checks (check_reference, status, defect_count, requires_rework, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Quality check ' || g::text,
  (array['Passed','Failed','Pending'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for quality checks.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Assets (2000 rows)
insert into pgapp_data.nexus_erp_assets (asset_name, status, asset_value, is_leased, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Asset ' || g::text,
  (array['InUse','InStorage','Disposed'])[1 + floor(random()*3)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for assets.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Locations (2000 rows)
insert into pgapp_data.nexus_erp_locations (location_name, status, square_footage, is_headquarters, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Location ' || g::text,
  (array['Active','Closed','Renovating'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for locations.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Maintenance requests (2000 rows)
insert into pgapp_data.nexus_erp_maintenance_requests (request_reference, status, priority_level, is_urgent, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Maintenance request ' || g::text,
  (array['Submitted','Scheduled','Completed'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for maintenance requests.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Reservations (2000 rows)
insert into pgapp_data.nexus_erp_reservations (reservation_reference, status, attendee_count, is_recurring, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Reservation ' || g::text,
  (array['Requested','Confirmed','Cancelled'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for reservations.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Policies (2000 rows)
insert into pgapp_data.nexus_erp_policies (policy_name, status, version_number, is_mandatory, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Policie ' || g::text,
  (array['Draft','Active','UnderReview','Retired'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for policies.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Audit logs (5000 rows — hot table)
insert into pgapp_data.nexus_erp_audit_logs (audit_reference, status, finding_count, is_high_risk, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Audit log ' || g::text,
  (array['Open','InReview','Closed'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for audit logs.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 5000) g;

-- Approvals (2000 rows)
insert into pgapp_data.nexus_erp_approvals (approval_reference, status, approval_level, is_escalated, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Approval ' || g::text,
  (array['Pending','Approved','Rejected'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for approvals.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Notifications (2000 rows)
insert into pgapp_data.nexus_erp_notifications (notification_title, status, priority_score, is_broadcast, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Notification ' || g::text,
  (array['Unread','Read','Archived'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  'Seed record ' || g::text || ' for notifications.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Contacts (40000 rows — hot table)
with pool_contacts as (
  select array_agg(company_name) as vals, count(*) as n from pgapp_data.nexus_erp_accounts
)
insert into pgapp_data.nexus_erp_contacts (full_name, status, is_primary, account_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Contact ' || g::text,
  (array['Lead','Qualified','Customer'])[1 + floor(random()*3)::int],
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_contacts),
  'Seed record ' || g::text || ' for contacts.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 40000) g;

-- Sales orders (15000 rows — hot table)
with pool_sales_orders as (
  select array_agg(quote_number) as vals, count(*) as n from pgapp_data.nexus_erp_quotes
)
insert into pgapp_data.nexus_erp_sales_orders (order_number, status, order_total, is_rush, linked_quote, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Sales order ' || g::text,
  (array['Pending','Confirmed','Shipped','Delivered'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_sales_orders),
  'Seed record ' || g::text || ' for sales orders.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 15000) g;

-- Stock levels (2000 rows)
with pool_stock_levels as (
  select array_agg(product_name) as vals, count(*) as n from pgapp_data.nexus_erp_products
)
insert into pgapp_data.nexus_erp_stock_levels (sku, status, quantity_on_hand, needs_reorder, product_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Stock level ' || g::text,
  (array['InStock','LowStock','OutOfStock'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_stock_levels),
  'Seed record ' || g::text || ' for stock levels.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Purchase orders (2000 rows)
with pool_purchase_orders as (
  select array_agg(supplier_name) as vals, count(*) as n from pgapp_data.nexus_erp_suppliers
)
insert into pgapp_data.nexus_erp_purchase_orders (po_number, status, po_total, is_urgent, supplier_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Purchase order ' || g::text,
  (array['Draft','Submitted','Approved','Received'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_purchase_orders),
  'Seed record ' || g::text || ' for purchase orders.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Positions (2000 rows)
with pool_positions as (
  select array_agg(department_name) as vals, count(*) as n from pgapp_data.nexus_erp_departments
)
insert into pgapp_data.nexus_erp_positions (title, status, pay_grade, is_remote, department_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Position ' || g::text,
  (array['Open','Filled','Frozen'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_positions),
  'Seed record ' || g::text || ' for positions.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- AR invoices (15000 rows — hot table)
with pool_ar_invoices as (
  select array_agg(account_name) as vals, count(*) as n from pgapp_data.nexus_erp_ledger_accounts
)
insert into pgapp_data.nexus_erp_ar_invoices (invoice_number, status, outstanding_amount, is_disputed, linked_account, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' AR invoice ' || g::text,
  (array['Open','PartiallyPaid','Paid','WrittenOff'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_ar_invoices),
  'Seed record ' || g::text || ' for ar invoices.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 15000) g;

-- Tasks (2000 rows)
with pool_tasks as (
  select array_agg(project_name) as vals, count(*) as n from pgapp_data.nexus_erp_projects
)
insert into pgapp_data.nexus_erp_tasks (task_name, status, estimated_hours, is_blocked, project_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Task ' || g::text,
  (array['Todo','InProgress','Review','Done'])[1 + floor(random()*4)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_tasks),
  'Seed record ' || g::text || ' for tasks.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Escalations (2000 rows)
with pool_escalations as (
  select array_agg(subject) as vals, count(*) as n from pgapp_data.nexus_erp_tickets
)
insert into pgapp_data.nexus_erp_escalations (escalation_reference, status, severity_level, is_customer_facing, escalated_ticket, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Escalation ' || g::text,
  (array['Raised','InReview','Resolved'])[1 + floor(random()*3)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_escalations),
  'Seed record ' || g::text || ' for escalations.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Marketing leads (2000 rows)
with pool_marketing_leads as (
  select array_agg(campaign_name) as vals, count(*) as n from pgapp_data.nexus_erp_campaigns
)
insert into pgapp_data.nexus_erp_marketing_leads (lead_name, status, lead_score, is_mql, campaign_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Marketing lead ' || g::text,
  (array['New','Nurturing','Qualified','Disqualified'])[1 + floor(random()*4)::int],
  1 + floor(random()*5)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_marketing_leads),
  'Seed record ' || g::text || ' for marketing leads.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Maintenance logs (2000 rows)
with pool_maintenance_logs as (
  select array_agg(machine_name) as vals, count(*) as n from pgapp_data.nexus_erp_machines
)
insert into pgapp_data.nexus_erp_maintenance_logs (log_reference, status, downtime_minutes, is_unplanned, machine_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Maintenance log ' || g::text,
  (array['Scheduled','InProgress','Completed'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_maintenance_logs),
  'Seed record ' || g::text || ' for maintenance logs.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Contracts (2000 rows)
with pool_contracts as (
  select array_agg(location_name) as vals, count(*) as n from pgapp_data.nexus_erp_locations
)
insert into pgapp_data.nexus_erp_contracts (contract_name, status, contract_value, is_auto_renew, location_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Contract ' || g::text,
  (array['Draft','Active','Expired','Terminated'])[1 + floor(random()*4)::int],
  1000 + floor(random()*499001)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_contracts),
  'Seed record ' || g::text || ' for contracts.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- Documents (2000 rows)
with pool_documents as (
  select array_agg(policy_name) as vals, count(*) as n from pgapp_data.nexus_erp_policies
)
insert into pgapp_data.nexus_erp_documents (document_title, status, page_count, is_confidential, policy_name, notes, created_at)
select
  (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' ' || (array['Atlas','Nova','Zenith','Orion','Vertex','Summit','Beacon','Cobalt','Delta','Ember','Falcon','Granite','Harbor','Indigo','Juniper','Keystone','Lumen','Meridian','Nimbus','Onyx','Pinnacle','Quartz','Ridge','Solstice','Talon','Umbra','Vantage','Willow','Xenon','Yield','Zephyr'])[1 + floor(random()*31)::int] || ' Document ' || g::text,
  (array['Draft','Final','Expired'])[1 + floor(random()*3)::int],
  1 + floor(random()*500)::int,
  (random() < 0.3),
  (select vals[1 + floor(random()*n)::int] from pool_documents),
  'Seed record ' || g::text || ' for documents.',
  now() - (floor(random()*180)::int || ' days')::interval
from generate_series(1, 2000) g;

-- total rows across 60 tables: 318000
