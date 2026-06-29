{{ config(materialized='incremental') }}

select
  id,
  topic,
  created_ms
from {{ source('substrate', 'memory') }}
where created_ms >= 2000
{% if is_incremental() %}
  and created_ms >= 3000
{% endif %}
