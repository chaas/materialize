# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

#
# Test domain types

> CREATE SECRET pgpass AS 'postgres'
> CREATE CONNECTION pg TO POSTGRES (
    HOST postgres,
    DATABASE postgres,
    USER postgres,
    PASSWORD SECRET pgpass
  )


# Insert data pre-snapshot
$ postgres-execute connection=postgres://postgres:postgres@postgres
ALTER USER postgres WITH replication;
DROP SCHEMA IF EXISTS public CASCADE;
DROP PUBLICATION IF EXISTS mz_source;

CREATE SCHEMA public;

CREATE DOMAIN posint AS integer CHECK (VALUE > 0);

CREATE TABLE t1 (f1 POSINT);
ALTER TABLE t1 REPLICA IDENTITY FULL;

INSERT INTO t1 VALUES (1);
INSERT INTO t1 VALUES (2);

CREATE PUBLICATION mz_source FOR ALL TABLES;

> CREATE SOURCE mz_source FROM POSTGRES CONNECTION pg (PUBLICATION 'mz_source');

! CREATE TABLE t1 FROM SOURCE mz_source (REFERENCE t1);
contains:referenced tables use unsupported types
