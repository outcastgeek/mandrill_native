use sqlx::{self, ConnectOptions, Row, types};
use sqlx_turso::{Turso, TursoConnectOptions};
use std::str::FromStr;

#[derive(Debug, Eq, PartialEq, sqlx::FromRow)]
struct DerivedRow {
    id: UserId,
    name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, sqlx::Type)]
#[sqlx(transparent)]
struct UserId(i64);

#[tokio::test]
async fn supported_types_and_stock_derives_round_trip() -> sqlx::Result<()> {
    let mut connection = TursoConnectOptions::new().connect().await?;
    sqlx::query::<Turso>(
        "CREATE TABLE values_table (\
         signed INTEGER, unsigned INTEGER, boolean BOOLEAN, real REAL, text TEXT, blob BLOB, \
         uuid BLOB, json TEXT, chrono_datetime DATETIME, time_datetime DATETIME\
         )",
    )
    .execute(&mut connection)
    .await?;

    let uuid = types::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    let json = "{\"usable\":true}"
        .parse::<types::JsonValue>()
        .map_err(sqlx::Error::config)?;
    let chrono_date = types::chrono::NaiveDate::from_ymd_opt(2026, 9, 13)
        .ok_or_else(|| sqlx::Error::Protocol("invalid chrono date fixture".to_owned()))?;
    let chrono_datetime = chrono_date
        .and_hms_micro_opt(12, 34, 56, 789_123)
        .ok_or_else(|| sqlx::Error::Protocol("invalid chrono time fixture".to_owned()))?;
    let time_date = types::time::Date::from_ordinal_date(2026, 256).map_err(sqlx::Error::config)?;
    let time_value =
        types::time::Time::from_hms_micro(12, 34, 56, 789_123).map_err(sqlx::Error::config)?;
    let time_datetime = types::time::PrimitiveDateTime::new(time_date, time_value);
    let borrowed_text = String::from("borrowed text");
    let borrowed_blob = vec![0_u8, 1, 2, 255];

    let max_unsigned = u64::try_from(i64::MAX).map_err(sqlx::Error::config)?;
    sqlx::query::<Turso>("INSERT INTO values_table VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(i64::MIN)
        .bind(max_unsigned)
        .bind(true)
        .bind(42.5_f64)
        .bind(borrowed_text.as_str())
        .bind(borrowed_blob.as_slice())
        .bind(uuid)
        .bind(types::Json(json.clone()))
        .bind(chrono_datetime)
        .bind(time_datetime)
        .execute(&mut connection)
        .await?;

    let row = sqlx::query::<Turso>("SELECT * FROM values_table")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<i64, _>("signed")?, i64::MIN);
    assert_eq!(row.try_get::<u64, _>("unsigned")?, max_unsigned);
    assert!(row.try_get::<bool, _>("boolean")?);
    assert_eq!(row.try_get::<f64, _>("real")?, 42.5);
    assert_eq!(row.try_get::<&str, _>("text")?, borrowed_text);
    assert_eq!(row.try_get::<&[u8], _>("blob")?, borrowed_blob);
    assert_eq!(row.try_get::<types::Uuid, _>("uuid")?, uuid);
    assert_eq!(
        row.try_get::<types::Json<types::JsonValue>, _>("json")?.0,
        json
    );
    assert_eq!(
        row.try_get::<types::chrono::NaiveDateTime, _>("chrono_datetime")?,
        chrono_datetime
    );
    assert_eq!(
        row.try_get::<types::time::PrimitiveDateTime, _>("time_datetime")?,
        time_datetime
    );

    let bytes = types::BString::from(vec![9_u8, 8, 7]);
    let row = sqlx::query::<Turso>("SELECT ? AS bytes")
        .bind(bytes.clone())
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<types::BString, _>("bytes")?, bytes);

    let big_decimal = types::BigDecimal::from_str("12345678901234567890.12345678901234567890")
        .map_err(sqlx::Error::config)?;
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(big_decimal.clone())
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<types::BigDecimal, _>("value")?, big_decimal);

    let decimal =
        types::Decimal::from_str("7922816251426433759354395.0335").map_err(sqlx::Error::config)?;
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(decimal)
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<types::Decimal, _>("value")?, decimal);

    let ip_net =
        types::ipnet::IpNet::from_str("2001:db8:1234::/48").map_err(sqlx::Error::config)?;
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(ip_net)
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<types::ipnet::IpNet, _>("value")?, ip_net);

    let ip_network =
        types::ipnetwork::IpNetwork::from_str("192.0.2.0/27").map_err(sqlx::Error::config)?;
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(ip_network)
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(
        row.try_get::<types::ipnetwork::IpNetwork, _>("value")?,
        ip_network
    );

    let mac_address = types::mac_address::MacAddress::from_str("02:00:5e:10:00:00")
        .map_err(sqlx::Error::config)?;
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(mac_address)
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(
        row.try_get::<types::mac_address::MacAddress, _>("value")?,
        mac_address
    );

    let bits = [true, false, true, true, false, false, true]
        .into_iter()
        .collect::<types::BitVec>();
    let row = sqlx::query::<Turso>("SELECT ? AS value")
        .bind(bits.clone())
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(row.try_get::<types::BitVec, _>("value")?, bits);

    let derived = sqlx::query_as::<Turso, DerivedRow>("SELECT 7 AS id, 'derived' AS name")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(
        derived,
        DerivedRow {
            id: UserId(7),
            name: "derived".to_owned(),
        }
    );

    Ok(())
}
