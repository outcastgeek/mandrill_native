use sqlx::{
    Acquire, Arguments, Connection, Executor, FromRow, QueryBuilder, Row, query, query_as,
    query_as_with, query_scalar, query_scalar_with, query_with,
};
use sqlx_turso::{Turso, TursoArguments, TursoConnection};

#[derive(Debug, Eq, FromRow, PartialEq)]
struct Item {
    id: i64,
    name: String,
}

#[tokio::test]
async fn ordinary_query_variants_execute_on_turso() -> sqlx::Result<()> {
    let mut connection = TursoConnection::connect("turso::memory:").await?;
    connection
        .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await?;

    let mut insert_arguments = TursoArguments::default();
    insert_arguments.add(1_i64).map_err(sqlx::Error::Encode)?;
    insert_arguments.add("alpha").map_err(sqlx::Error::Encode)?;
    query_with::<Turso, _>(
        "INSERT INTO items (id, name) VALUES (?, ?)",
        insert_arguments,
    )
    .execute(&mut connection)
    .await?;

    query::<Turso>("INSERT INTO items (id, name) VALUES (?, ?)")
        .bind(2_i64)
        .bind("beta")
        .execute(&mut connection)
        .await?;

    let first = query_as::<Turso, Item>("SELECT id, name FROM items WHERE id = ?")
        .bind(1_i64)
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(
        first,
        Item {
            id: 1,
            name: "alpha".to_owned(),
        }
    );

    let mut select_arguments = TursoArguments::default();
    select_arguments.add(2_i64).map_err(sqlx::Error::Encode)?;
    let second = query_as_with::<Turso, Item, _>(
        "SELECT id, name FROM items WHERE id = ?",
        select_arguments,
    )
    .fetch_one(&mut connection)
    .await?;
    assert_eq!(second.name, "beta");

    let count = query_scalar::<Turso, i64>("SELECT COUNT(*) FROM items")
        .fetch_one(&mut connection)
        .await?;
    assert_eq!(count, 2);

    let mut scalar_arguments = TursoArguments::default();
    scalar_arguments.add(2_i64).map_err(sqlx::Error::Encode)?;
    let selected =
        query_scalar_with::<Turso, i64, _>("SELECT id FROM items WHERE id = ?", scalar_arguments)
            .fetch_one(&mut connection)
            .await?;
    assert_eq!(selected, 2);

    Ok(())
}

#[tokio::test]
async fn query_builder_mapping_and_fetch_shapes_work_on_turso() -> sqlx::Result<()> {
    let mut connection = TursoConnection::connect("turso::memory:").await?;
    connection
        .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await?;

    let mut insert = QueryBuilder::<Turso>::new("INSERT INTO items (id, name) ");
    insert.push_values(
        [(1_i64, "alpha"), (2_i64, "beta"), (3_i64, "gamma")],
        |mut row, item| {
            row.push_bind(item.0).push_bind(item.1);
        },
    );
    assert_eq!(
        insert
            .build()
            .execute(&mut connection)
            .await?
            .rows_affected(),
        3
    );

    let names = query::<Turso>("SELECT name FROM items ORDER BY id")
        .map(|row| row.get::<String, _>("name"))
        .fetch_all(&mut connection)
        .await?;
    assert_eq!(names, ["alpha", "beta", "gamma"]);

    let ids = query::<Turso>("SELECT id FROM items ORDER BY id")
        .try_map(|row| row.try_get::<i64, _>("id"))
        .fetch_all(&mut connection)
        .await?;
    assert_eq!(ids, [1, 2, 3]);

    assert!(
        query::<Turso>("SELECT id FROM items WHERE id = 99")
            .fetch_optional(&mut connection)
            .await?
            .is_none()
    );
    assert!(matches!(
        query::<Turso>("SELECT id FROM items WHERE id = 99")
            .fetch_one(&mut connection)
            .await,
        Err(sqlx::Error::RowNotFound)
    ));

    Ok(())
}

#[tokio::test]
async fn borrowed_text_and_blob_bind_without_copying_the_api_surface() -> sqlx::Result<()> {
    let mut connection = TursoConnection::connect("turso::memory:").await?;
    let text = String::from("borrowed");
    let blob = vec![1_u8, 2, 3, 4];

    let row = query::<Turso>("SELECT ? AS text_value, ? AS blob_value")
        .bind(text.as_str())
        .bind(blob.as_slice())
        .fetch_one(&mut connection)
        .await?;

    assert_eq!(row.try_get::<&str, _>("text_value")?, "borrowed");
    assert_eq!(row.try_get::<&[u8], _>("blob_value")?, [1, 2, 3, 4]);
    Ok(())
}

#[tokio::test]
async fn mutable_connection_satisfies_generic_acquire() -> sqlx::Result<()> {
    async fn read_one<'connection, A>(acquire: A) -> sqlx::Result<i64>
    where
        A: Acquire<'connection, Database = Turso>,
    {
        let mut connection = acquire.acquire().await?;
        query_scalar::<Turso, i64>("SELECT 1")
            .fetch_one(&mut *connection)
            .await
    }

    let mut connection = TursoConnection::connect("turso::memory:").await?;
    assert_eq!(read_one(&mut connection).await?, 1);
    Ok(())
}
