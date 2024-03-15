# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

$ set-arg-default replicas=1
$ set-arg-default single-replica-cluster=quickstart

# Ideally we'd have separate replacements for user and system ids,
# but testdrive only supports a single set-regex statement.
$ set-regex match=(u|s)\d+ replacement=<GID>

# Testdrive accounts for multi-replica runs by creating additional clusters,
# which affect the output of this test––instead just don't run this test.

$ skip-if
SELECT ${arg.replicas} > 1;

$ set-sql-timeout duration=1s

# Test \d <object> command
> CREATE TABLE tbl (a int, b text)

> CREATE INDEX tbl_ind ON tbl (b)

> CREATE INDEX tbl_lower_ind ON tbl (lower(b), a)

$ psql-execute command="\d tbl"
\                Table "public.tbl"
 Column |  Type   | Collation | Nullable | Default
--------+---------+-----------+----------+---------
 a      | integer |           |          | NULL
 b      | text    |           |          | NULL
Indexes:
    "tbl_ind" arrangement (b)
    "tbl_lower_ind" arrangement (pg_catalog.lower(b),a)

> DROP TABLE tbl CASCADE

# What schemas do we have by default?
> SHOW SCHEMAS FROM materialize
public              <GID>
information_schema  <GID>
mz_catalog          <GID>
mz_unsafe           <GID>
mz_internal         <GID>
pg_catalog          <GID>
> SHOW SCHEMAS
name               id
-------------------------
information_schema <GID>
public             <GID>
mz_catalog         <GID>
mz_unsafe          <GID>
mz_internal        <GID>
pg_catalog         <GID>
$ psql-execute command=\dn
\     List of schemas
    Name     |   Owner
-------------+-----------
 mz_catalog  | mz_system
 mz_internal | mz_system
 mz_unsafe   | mz_system
 public      | mz_system


$ psql-execute command="\dn mz_catalog"
\    List of schemas
    Name    |   Owner
------------+-----------
 mz_catalog | mz_system


$ psql-execute command="\dn mz_*"
\     List of schemas
    Name     |   Owner
-------------+-----------
 mz_catalog  | mz_system
 mz_internal | mz_system
 mz_unsafe   | mz_system

# What objects do we have by default?
> SHOW OBJECTS
name    id    type
-------------------

# Creating a schema should be reflected in the output of SHOW SCHEMAS.
> CREATE SCHEMA s
> SHOW SCHEMAS
name                id
--------------------------
public              <GID>
s                   <GID>
information_schema  <GID>
mz_catalog          <GID>
mz_internal         <GID>
mz_unsafe           <GID>
pg_catalog          <GID>


# Creating a schema with a name that already exists should fail.
! CREATE SCHEMA s
contains:schema 's' already exists

# Dropping a schema with a view should only succeed with CASCADE.
> CREATE VIEW s.v AS SELECT 1
! DROP SCHEMA s
contains:schema 'materialize.s' cannot be dropped without CASCADE while it contains objects
> DROP SCHEMA s CASCADE

# Dropping a schema with no objects should succeed without CASCADE.
> CREATE SCHEMA s
> CREATE VIEW s.v AS SELECT 1
> DROP VIEW s.v
> DROP SCHEMA s

# What databases do we have by default?
> SHOW DATABASES
name          id
--------------------
materialize   <GID>

> SELECT name FROM mz_databases
name
-----------
materialize

# Creating a database should be reflected in the output of SHOW DATABASES.
> CREATE DATABASE d
> SHOW DATABASES
name          id
--------------------
d             <GID>
materialize   <GID>

# ...and also in `\l`
$ psql-execute command="\l"
\                             List of databases
    Name     |    Owner    | Encoding | Collate | Ctype | Access privileges
-------------+-------------+----------+---------+-------+-------------------
 d           | materialize | UTF8     | C       | C     |
 materialize | mz_system   | UTF8     | C       | C     |



> SELECT name FROM mz_databases
name
-----------
materialize
d

# The same catalog information should be accessible with any amount of
# database or schema qualification.
> SELECT count(*) FROM materialize.mz_catalog.mz_databases
2
> SELECT count(*) FROM d.mz_catalog.mz_databases
2

# SHOW DATABASES should filter its output according to the provided LIKE or
# WHERE clause.
> SHOW DATABASES LIKE 'foo'
> SHOW DATABASES LIKE 'd'
d   <GID>
> SHOW DATABASES LIKE 'mat%'
materialize   <GID>
> SHOW DATABASES WHERE (SELECT name = name)
materialize   <GID>
d             <GID>
> SHOW DATABASES WHERE (name = (SELECT min(name) FROM mz_databases))
d   <GID>
! SHOW DATABASES WHERE 7
contains:WHERE clause must have type boolean, not type integer

# Creating a database with a name that already exists should fail.
! CREATE DATABASE d
contains:database 'd' already exists

# The new database should have a default public schema.
> SHOW SCHEMAS FROM d
name                id
--------------------------
public              <GID>
information_schema  <GID>
mz_catalog          <GID>
mz_internal         <GID>
mz_unsafe           <GID>
pg_catalog          <GID>

# New schemas in the database should appear in the output of SHOW SCHEMAS FROM.
> CREATE SCHEMA d.s
> SHOW SCHEMAS FROM d
name                id
--------------------------
public              <GID>
s                   <GID>
information_schema  <GID>
mz_catalog          <GID>
mz_internal         <GID>
mz_unsafe           <GID>
pg_catalog          <GID>

# SHOW SCHEMAS should filter its output based on the provided LIKE or WHERE
# clause.
> SHOW SCHEMAS LIKE 'pub%'
public    <GID>
> SHOW SCHEMAS LIKE 'private'
> SHOW SCHEMAS WHERE name = 'public'
public    <GID>

# New views in the database should work.
> CREATE VIEW d.public.v1 AS SELECT 1
> SHOW VIEWS FROM d.public
name  id
------------
v1    <GID>

# Setting the session database should update name resolution appropriately.
> SET DATABASE = d
> SHOW DATABASE
d
> SHOW SCHEMAS
name                id
--------------------------
public              <GID>
s                   <GID>
information_schema  <GID>
mz_catalog          <GID>
mz_internal         <GID>
mz_unsafe           <GID>
pg_catalog          <GID>
> CREATE VIEW v2 AS SELECT 2
> SHOW VIEWS
v1  <GID>
v2  <GID>
> SELECT * FROM v1 UNION ALL SELECT * FROM v2
1
2

> SHOW OBJECTS
name    id      type
----------------------
v1      <GID>   view    
v2      <GID>   view    

# Test minimizing name qualification

> CREATE TYPE int_list AS list (ELEMENT TYPE = int4)

> SELECT pg_typeof('{1}'::d.public.int_list)
int_list

> CREATE SCHEMA other
> CREATE TYPE other.int_list AS list (ELEMENT TYPE = int4)
> SELECT pg_typeof('{1}'::d.other.int_list)
other.int_list

> CREATE DATABASE foo
> CREATE SCHEMA foo.other
> CREATE TYPE foo.other.int_list AS LIST (ELEMENT TYPE = int4)
> SELECT pg_typeof('{1}'::foo.other.int_list)
foo.other.int_list

> CREATE TYPE bool AS LIST (ELEMENT TYPE = int4)
! SELECT '{1}'::bool
contains:invalid input syntax for type boolean: "{1}"

> SELECT pg_typeof('{1}'::public.bool);
public.bool

> SELECT pg_typeof('{1}'::d.public.bool);
public.bool

> DROP DATABASE foo

! DROP OBJECT v1
contains:Expected one of TABLE or VIEW or MATERIALIZED or SOURCE or SINK or INDEX or TYPE or ROLE or USER or CLUSTER or SECRET or CONNECTION or DATABASE or SCHEMA or FUNCTION, found identifier

> SHOW OBJECTS
name      id      type
----------------------
bool      <GID>   type
int_list  <GID>   type
v1        <GID>   view    
v2        <GID>   view 

# Create one of every mz_object type
$ set schema={
    "name": "row",
    "type": "record",
    "fields": [
      {"name": "a", "type": "long"}
    ]
  }
$ kafka-create-topic topic=data
$ kafka-ingest format=avro topic=data schema=${schema} timestamp=1
{"a": 1}

> CREATE TABLE tbl (a int, b text);
> CREATE SECRET pass_secret AS 'pass';

> CREATE CONNECTION IF NOT EXISTS csr_conn TO CONFLUENT SCHEMA REGISTRY (
    URL '${testdrive.schema-registry-url}'
  );

> CREATE MATERIALIZED VIEW mv AS SELECT (1);

> CREATE CONNECTION kafka_conn
  TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT);

> CREATE SOURCE source_data
  IN CLUSTER ${arg.single-replica-cluster}
  FROM KAFKA CONNECTION kafka_conn (TOPIC 'testdrive-data-${testdrive.seed}')
  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn;
> CREATE SINK snk
  IN CLUSTER ${arg.single-replica-cluster}
  FROM source_data
  INTO KAFKA CONNECTION kafka_conn (TOPIC 'testdrive-catalog-sink-${testdrive.seed}')
  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn
  ENVELOPE DEBEZIUM;

> SHOW OBJECTS
name        id    type
-------------------------------------
bool        <GID> type
csr_conn    <GID> connection
int_list    <GID> type
v1          <GID> view
v2          <GID> view
tbl         <GID> table
pass_secret <GID> secret
kafka_conn  <GID> connection
mv          <GID> materialized-view
source_data <GID> source
source_data_progress <GID> source
snk         <GID> sink

> SELECT DISTINCT(TYPE) FROM mz_objects
type
----
table
source
view
materialized-view
sink
index
connection
type
function
secret

> SELECT * FROM (SHOW OBJECTS) ORDER BY name DESC
bool        <GID>   type
csr_conn    <GID>   connection
int_list    <GID>   type
kafka_conn  <GID>   connection
mv          <GID>   materialized-view
pass_secret <GID>   secret
snk         <GID>   sink
source_data <GID>   source
source_data_progress <GID>   source
tbl         <GID>   table
v1          <GID>   view
v2          <GID>   view

> SELECT create_sql FROM (SHOW CREATE TABLE tbl)
"CREATE TABLE \"d\".\"public\".\"tbl\" (\"a\" \"pg_catalog\".\"int4\", \"b\" \"pg_catalog\".\"text\")"

! SHOW COLUMNS FROM pass_secret
contains:d.public.pass_secret is a secret and so does not have columns
! SHOW COLUMNS FROM kafka_conn
contains:d.public.kafka_conn is a connection and so does not have columns

# DROP DATABASE does not support both RESTRICT and CASCADE.
! DROP DATABASE d RESTRICT CASCADE
contains:Cannot specify both RESTRICT and CASCADE in DROP
! DROP DATABASE d CASCADE RESTRICT
contains:Cannot specify both CASCADE and RESTRICT in DROP
! DROP DATABASE d CASCADE CASCADE
contains:Expected end of statement, found CASCADE

! DROP DATABASE d RESTRICT
contains:database 'd' cannot be dropped with RESTRICT while it contains schemas

# DROP DATABASE should succeed even when there are objects in the database.
> DROP DATABASE d
# SHOW DATABASES should work, even if the current database has been dropped.
> SHOW DATABASES
name        id
-----------------
materialize <GID>

> SELECT name FROM mz_databases
name
-----------
materialize

# The session database should remain set to the dropped database, but future
# queries that depend on the session database should fail with sensible error
# messages.
> SHOW DATABASE
d
! SELECT * FROM v
contains:unknown catalog item 'v'

# But queries that do not depend on the session database should work fine.
> CREATE VIEW materialize.public.v AS SELECT 1
> CREATE DATABASE d

# Dropping the public schema is okay, but dropping the catalog schemas is not.
> DROP SCHEMA public

$ postgres-execute connection=postgres://mz_system@${testdrive.materialize-internal-sql-addr}/materialize
ALTER SYSTEM SET enable_rbac_checks TO false

! DROP SCHEMA mz_catalog
contains:cannot drop schema mz_catalog because it is required by the database system
! DROP SCHEMA pg_catalog
contains:cannot drop schema pg_catalog because it is required by the database system

$ postgres-execute connection=postgres://mz_system@${testdrive.materialize-internal-sql-addr}/materialize
ALTER SYSTEM SET enable_rbac_checks TO true

# Schema names that start with "mz_" or "pg_" are reserved for future use by the
# system.
! CREATE SCHEMA mz_foo
contains:unacceptable schema name 'mz_foo'
! CREATE SCHEMA pg_bar
contains:unacceptable schema name 'pg_bar'

# The search path is configurable.
> SHOW search_path
"public"
> SET search_path = foo
> SET SCHEMA foo
> SET search_path to public

# Creating views in non-existent databases should fail.
! CREATE VIEW noexist.ignored AS SELECT 1
contains:unknown schema 'noexist'
! CREATE VIEW materialize.noexist.ignored AS SELECT 1
contains:unknown schema 'noexist'
! CREATE VIEW noexist.ignored.ignored AS SELECT 1
contains:unknown database 'noexist'

# As should showing views.
! SHOW VIEWS FROM noexist
contains:unknown schema 'noexist'
! SHOW VIEWS FROM noexist_db.noexist_schema
contains:unknown database 'noexist_db'

# Dropping database with cross-schema dependencies is ok.
> CREATE DATABASE d1;
> CREATE SCHEMA d1.s1;
> CREATE VIEW d1.s1.t as select 1;
> CREATE VIEW d1.public.tt as select * from d1.s1.t;
> DROP DATABASE d1;

# Dropping database with cross-database dependencies is ok and drops the
# dependent views.
> CREATE DATABASE d1;
> CREATE VIEW d1.public.t as select 1;
> CREATE DATABASE d2;
> CREATE VIEW d2.public.t AS SELECT * FROM d1.public.t;
> DROP DATABASE d1;
> SELECT name FROM SHOW DATABASES
name
-----------
d
d2
materialize
> SELECT name FROM mz_databases
name
-----------
materialize
d
d2

> SHOW VIEWS FROM d2.public;
 name   id
-----------

# Check default sources, tables, and views in mz_catalog.

> SHOW SOURCES FROM mz_catalog
name  id  type  size  cluster
------------------------------

> SHOW TABLES FROM mz_catalog
name                            id
-------------------------------------
mz_array_types	                <GID>
mz_audit_events	                <GID>
mz_aws_privatelink_connections	<GID>
mz_base_types	                  <GID>
mz_clusters	                    <GID>
mz_cluster_replicas		          <GID>
mz_columns		                  <GID>
mz_connections		              <GID>
mz_databases	                 	<GID>
mz_default_privileges	         	<GID>
mz_egress_ips	   	     	       	<GID>
mz_functions		     	  	      <GID>
mz_index_columns		     	      <GID>
mz_indexes                     	<GID>
mz_kafka_connections	  	  	  <GID>
mz_kafka_sinks	  	  	        <GID>
mz_list_types	  	  	          <GID>
mz_map_types	  	  	          <GID>
mz_materialized_views	  	  	  <GID>
mz_operators	  	  	          <GID>
mz_pseudo_types	  	  	        <GID>
mz_roles	  	  	    	    	  <GID>
mz_role_members	  	  	        <GID>
mz_schemas	  	  	    	  	  <GID>
mz_secrets	  	  	            <GID>
mz_sinks	  	  	              <GID>
mz_sources	  	  	            <GID>
mz_ssh_tunnel_connections	      <GID>
mz_system_privileges	          <GID>
mz_tables	  	  	    	  	    <GID>
mz_types	  	  	    	  	    <GID>
mz_views	  	  	    	  	    <GID>

> SHOW VIEWS FROM mz_catalog
name                      id
-----------------------------------
mz_objects                <GID>
mz_relations              <GID>
mz_storage_usage          <GID>
mz_timezone_abbreviations <GID>
mz_timezone_names         <GID>

# Check default sources, tables, and views in mz_internal.

> SHOW SOURCES FROM mz_internal
name                                         id     type   size  cluster
-------------------------------------------------------------------------
mz_active_peeks_per_worker                   <GID>	log   <null>   <null>
mz_arrangement_batcher_allocations_raw       <GID>	log   <null>   <null>
mz_arrangement_batcher_capacity_raw          <GID>	log   <null>   <null>
mz_arrangement_batcher_records_raw           <GID>	log   <null>   <null>
mz_arrangement_batcher_size_raw              <GID>	log   <null>   <null>
mz_arrangement_batches_raw                   <GID>	log   <null>   <null>
mz_arrangement_heap_allocations_raw          <GID>	log   <null>   <null>
mz_arrangement_heap_capacity_raw             <GID>	log   <null>   <null>
mz_arrangement_heap_size_raw                 <GID>	log   <null>   <null>
mz_arrangement_records_raw                   <GID>	log   <null>   <null>
mz_arrangement_sharing_raw                   <GID>	log   <null>   <null>
mz_aws_privatelink_connection_status_history <GID>	source <null>  <null>
mz_cluster_replica_frontiers                 <GID>	source <null>  <null>
mz_cluster_replica_heartbeats                <GID>	source <null>  <null>
mz_compute_delays_histogram_raw              <GID>	log   <null>   <null>
mz_compute_dependencies                      <GID>	source <null>  <null>
mz_compute_error_counts_raw                  <GID>	log   <null>   <null>
mz_compute_exports_per_worker                <GID>	log   <null>   <null>
mz_compute_frontiers_per_worker              <GID>	log   <null>   <null>
mz_compute_hydration_statuses                <GID>	source <null>  <null>
mz_compute_import_frontiers_per_worker       <GID>	log   <null>   <null>
mz_compute_operator_durations_histogram_raw  <GID>	log   <null>   <null>
mz_compute_operator_hydration_statuses_per_worker <GID>	source <null> <null>
mz_dataflow_addresses_per_worker             <GID>	log   <null>   <null>
mz_dataflow_channels_per_worker              <GID>	log   <null>   <null>
mz_dataflow_operator_reachability_raw        <GID>	log   <null>   <null>
mz_dataflow_operators_per_worker             <GID>	log   <null>   <null>
mz_dataflow_shutdown_durations_histogram_raw <GID>	log   <null>   <null>
mz_frontiers                                 <GID>	source <null>  <null>
mz_message_counts_received_raw               <GID>	log   <null>   <null>
mz_message_counts_sent_raw                   <GID>	log   <null>   <null>
mz_message_batch_counts_received_raw         <GID>	log   <null>   <null>
mz_message_batch_counts_sent_raw             <GID>	log   <null>   <null>
mz_peek_durations_histogram_raw              <GID>	log   <null>   <null>
mz_prepared_statement_history                <GID>	source <null>  <null>
mz_scheduling_elapsed_raw                    <GID>	log   <null>   <null>
mz_scheduling_parks_histogram_raw            <GID>	log   <null>   <null>
mz_session_history                           <GID>	source <null>  <null>
mz_sink_statistics_raw                       <GID>	source <null>  <null>
mz_sink_status_history                       <GID>	source <null>  <null>
mz_source_statistics_raw                     <GID>	source <null>  <null>
mz_source_status_history                     <GID>	source <null>  <null>
mz_sql_text                                  <GID>	source <null>  <null>
mz_statement_execution_history               <GID>	source <null>  <null>
mz_statement_lifecycle_history               <GID>	source <null>  <null>
mz_storage_shards                            <GID>	source <null>  <null>

> SHOW TABLES FROM mz_internal
name                          id
------------------------------------
mz_aggregates	                <GID>
mz_aws_connections	          <GID>
mz_cluster_replica_metrics	  <GID>
mz_cluster_replica_sizes      <GID>
mz_cluster_replica_statuses   <GID>
mz_comments	                  <GID>
mz_internal_cluster_replicas  <GID>
mz_kafka_sources	            <GID>
mz_object_dependencies	      <GID>
mz_optimizer_notices	        <GID>
mz_postgres_sources	          <GID>
mz_sessions	                  <GID>
mz_storage_usage_by_shard	    <GID>
mz_subscriptions              <GID>
mz_type_pg_metadata           <GID>
mz_webhook_sources            <GID>

> SHOW VIEWS FROM mz_internal
name                                                id
----------------------------------------------------------
mz_active_peeks                                     <GID>
mz_activity_log_thinned                             <GID>
mz_arrangement_sharing                              <GID>
mz_arrangement_sharing_per_worker                   <GID>
mz_arrangement_sizes                                <GID>
mz_arrangement_sizes_per_worker                     <GID>
mz_cluster_replica_history                          <GID>
mz_cluster_replica_utilization                      <GID>
mz_compute_delays_histogram                         <GID>
mz_compute_delays_histogram_per_worker              <GID>
mz_compute_error_counts                             <GID>
mz_compute_error_counts_per_worker                  <GID>
mz_compute_exports                                  <GID>
mz_compute_frontiers                                <GID>
mz_compute_import_frontiers                         <GID>
mz_compute_operator_durations_histogram             <GID>
mz_compute_operator_durations_histogram_per_worker  <GID>
mz_compute_operator_hydration_statuses              <GID>
mz_dataflow_addresses                               <GID>
mz_dataflow_arrangement_sizes                       <GID>
mz_dataflow_channel_operators                       <GID>
mz_dataflow_channel_operators_per_worker            <GID>
mz_dataflow_channels                                <GID>
mz_dataflow_operator_dataflows                      <GID>
mz_dataflow_operator_dataflows_per_worker           <GID>
mz_dataflow_operator_parents                        <GID>
mz_dataflow_operator_parents_per_worker             <GID>
mz_dataflow_operator_reachability                   <GID>
mz_dataflow_operator_reachability_per_worker        <GID>
mz_dataflow_operators                               <GID>
mz_dataflow_shutdown_durations_histogram            <GID>
mz_dataflow_shutdown_durations_histogram_per_worker <GID>
mz_dataflows                                        <GID>
mz_dataflows_per_worker                             <GID>
mz_expected_group_size_advice                       <GID>
mz_global_frontiers                                 <GID>
mz_hydration_statuses                               <GID>
mz_materialization_lag                              <GID>
mz_message_counts                                   <GID>
mz_message_counts_per_worker                        <GID>
mz_notices                                          <GID>
mz_notices_redacted                                 <GID>
mz_object_fully_qualified_names                     <GID>
mz_object_lifetimes                                 <GID>
mz_object_oid_alias                                 <GID>
mz_object_transitive_dependencies                   <GID>
mz_peek_durations_histogram                         <GID>
mz_peek_durations_histogram_per_worker              <GID>
mz_recent_activity_log                              <GID>
mz_recent_activity_log_thinned                      <GID>
mz_recent_activity_log_redacted                     <GID>
mz_recent_sql_text                                  <GID>
mz_recent_sql_text_redacted                         <GID>
mz_records_per_dataflow                             <GID>
mz_records_per_dataflow_operator                    <GID>
mz_records_per_dataflow_operator_per_worker         <GID>
mz_records_per_dataflow_per_worker                  <GID>
mz_scheduling_elapsed                               <GID>
mz_scheduling_elapsed_per_worker                    <GID>
mz_scheduling_parks_histogram                       <GID>
mz_scheduling_parks_histogram_per_worker            <GID>
mz_show_all_my_privileges                           <GID>
mz_show_all_privileges                              <GID>
mz_show_cluster_privileges                          <GID>
mz_show_cluster_replicas                            <GID>
mz_show_database_privileges                         <GID>
mz_show_default_privileges                          <GID>
mz_show_indexes                                     <GID>
mz_show_materialized_views                          <GID>
mz_show_my_cluster_privileges                       <GID>
mz_show_my_database_privileges                      <GID>
mz_show_my_default_privileges                       <GID>
mz_show_my_object_privileges                        <GID>
mz_show_my_role_members                             <GID>
mz_show_my_schema_privileges                        <GID>
mz_show_my_system_privileges                        <GID>
mz_show_object_privileges                           <GID>
mz_show_role_members                                <GID>
mz_show_schema_privileges                           <GID>
mz_show_sinks                                       <GID>
mz_show_sources                                     <GID>
mz_show_system_privileges                           <GID>
mz_sink_statistics                                  <GID>
mz_sink_statuses                                    <GID>
mz_source_statistics                                <GID>
mz_source_statuses                                  <GID>
mz_sql_text_redacted                                <GID>
mz_aws_privatelink_connection_statuses              <GID>
mz_statement_execution_history_redacted             <GID>

> SET database = materialize

> CREATE SCHEMA tester

> SHOW TABLES FROM tester

> CREATE TABLE tester.test_table (a int)

> SHOW TABLES FROM tester
test_table  <GID>

# `SHOW TABLES` and `mz_tables` should agree.
> SELECT COUNT(*) FROM mz_tables WHERE id LIKE 's%'
48

# There is one entry in mz_indexes for each field_number/expression of the index.
> SELECT COUNT(id) FROM mz_indexes WHERE id LIKE 's%'
127

# Create a second schema with the same table name as above
> CREATE SCHEMA tester2

> CREATE TABLE tester2.test_table (a int)

$ psql-execute command="\dt tester.*"
\             List of relations
 Schema |    Name    | Type  |    Owner
--------+------------+-------+-------------
 tester | test_table | table | materialize

$ psql-execute command="\dt tester.test_table"
\             List of relations
 Schema |    Name    | Type  |    Owner
--------+------------+-------+-------------
 tester | test_table | table | materialize

$ psql-execute command="\dt *.test_table"
\             List of relations
 Schema  |    Name    | Type  |    Owner
---------+------------+-------+-------------
 tester  | test_table | table | materialize
 tester2 | test_table | table | materialize

> CREATE TYPE type1 AS LIST (ELEMENT TYPE = text)

> SHOW TYPES
type1

$ psql-execute command="\dT"
\              List of data types
   Schema   |       Name        | Description
------------+-------------------+-------------
 mz_catalog | anycompatiblelist |
 mz_catalog | anycompatiblemap  |
 mz_catalog | list              |
 mz_catalog | map               |
 mz_catalog | mz_aclitem        |
 mz_catalog | mz_timestamp      |
 mz_catalog | uint2             |
 mz_catalog | uint4             |
 mz_catalog | uint8             |
 public     | type1             |
