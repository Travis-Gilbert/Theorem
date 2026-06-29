{{ config(materialized='table') }}

select
  id,
  topic,
  created_ms
from {{ source('substrate', 'memory') }}
