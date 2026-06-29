{{ config(materialized='view') }}

select
  topic,
  count(*) as n_records
from {{ source('substrate', 'memory') }}
group by topic
