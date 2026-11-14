use std::any::type_name;
use std::sync::Arc;

use datafusion::arrow::array::{
    Array, Float64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use domain::{DomainError, MacAddress, Measurement, Metric, SourceKind, Timestamp};

use crate::StorageError;

/// The columns of a day file. The metric and the date come from its path.
pub(crate) fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("source_kind", DataType::Utf8, false),
        Field::new("source_id", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]))
}

pub(crate) fn to_batch(rows: &[Measurement]) -> Result<RecordBatch, StorageError> {
    let timestamps = TimestampMicrosecondArray::from_iter_values(
        rows.iter().map(|row| row.timestamp.as_micros()),
    )
    .with_timezone("UTC");
    let kinds = StringArray::from_iter_values(rows.iter().map(|row| row.source_kind.as_str()));
    let ids = StringArray::from_iter_values(rows.iter().map(|row| row.source_id.as_str()));
    let values = Float64Array::from_iter_values(rows.iter().map(|row| row.value));
    Ok(RecordBatch::try_new(
        schema(),
        vec![
            Arc::new(timestamps),
            Arc::new(kinds),
            Arc::new(ids),
            Arc::new(values),
        ],
    )?)
}

/// Rebuilds measurements of `metric` from `batch`, finding columns by name.
pub(crate) fn from_batch(
    batch: &RecordBatch,
    metric: Metric,
) -> Result<Vec<Measurement>, StorageError> {
    let timestamps = column::<TimestampMicrosecondArray>(batch, "timestamp")?;
    let kinds = column::<StringArray>(batch, "source_kind")?;
    let ids = column::<StringArray>(batch, "source_id")?;
    let values = column::<Float64Array>(batch, "value")?;
    (0..batch.num_rows())
        .map(|row| {
            let source_kind = kinds.value(row).parse::<SourceKind>().map_err(invalid)?;
            let mac = ids.value(row).parse::<MacAddress>().map_err(invalid)?;
            Ok(Measurement {
                timestamp: Timestamp::from_micros(timestamps.value(row)),
                source_kind,
                source_id: mac.into(),
                metric,
                value: values.value(row),
            })
        })
        .collect()
}

fn invalid(error: DomainError) -> StorageError {
    StorageError::InvalidColumn(error.to_string())
}

fn column<'a, T: Array + 'static>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a T, StorageError> {
    batch
        .column_by_name(name)
        .and_then(|array| array.as_any().downcast_ref::<T>())
        .ok_or_else(|| StorageError::InvalidColumn(format!("{name} is not a {}", type_name::<T>())))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::array::{
        Float64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
    };
    use domain::{MacAddress, Measurement, Metric, SourceKind, Timestamp};

    use super::{from_batch, schema, to_batch};
    use crate::StorageError;

    fn reading(micros: i64, value: f64) -> Measurement {
        Measurement {
            timestamp: Timestamp::from_micros(micros),
            source_kind: SourceKind::Ble,
            source_id: MacAddress::new([1, 2, 3, 4, 5, 6]).into(),
            metric: Metric::Humidity,
            value,
        }
    }

    #[test]
    fn rows_survive_a_batch_round_trip() {
        let rows = vec![reading(1, 1.5), reading(2, 2.5)];
        let batch = to_batch(&rows).expect("batch");
        assert_eq!(from_batch(&batch, Metric::Humidity).expect("rows"), rows);
    }

    #[test]
    fn an_unknown_source_kind_is_an_invalid_column() {
        let batch = RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(TimestampMicrosecondArray::from(vec![1]).with_timezone("UTC")),
                Arc::new(StringArray::from(vec!["zigbee"])),
                Arc::new(StringArray::from(vec!["01:02:03:04:05:06"])),
                Arc::new(Float64Array::from(vec![1.0])),
            ],
        )
        .expect("batch");
        assert!(matches!(
            from_batch(&batch, Metric::Humidity),
            Err(StorageError::InvalidColumn(_))
        ));
    }
}
