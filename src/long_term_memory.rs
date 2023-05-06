use crate::models::{parse_redisearch_response, MemoryMessage, RedisearchResult};
use redis::Value;

use async_openai::{types::CreateEmbeddingRequestArgs, Client};
use byteorder::{LittleEndian, WriteBytesExt};
use nanoid::nanoid;
use std::io::Cursor;

fn encode(fs: Vec<f32>) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    for f in fs {
        buf.write_f32::<LittleEndian>(f).unwrap();
    }
    buf.into_inner()
}

pub async fn index_messages(
    messages: Vec<MemoryMessage>,
    session_id: String,
    openai_client: Client,
    mut redis_conn: redis::aio::ConnectionManager,
) -> Result<(), Box<dyn std::error::Error>> {
    let contents: Vec<String> = messages.iter().map(|msg| msg.content.clone()).collect();

    let request = CreateEmbeddingRequestArgs::default()
        .model("text-embedding-ada-002")
        .input(contents.clone())
        .build()?;

    let response = openai_client.embeddings().create(request).await?;

    // TODO add used tokens let tokens_used = response.usage.total_tokens;
    for data in response.data {
        let id = nanoid!();
        let key = format!("motorhead:{}", id);
        let vector = encode(data.embedding);

        redis::cmd("HSET")
            .arg(key)
            .arg("session")
            .arg(&session_id)
            .arg("vector")
            .arg(vector)
            .arg("content")
            .arg(&contents[data.index as usize])
            .arg("role")
            .arg(&messages[data.index as usize].role)
            .query_async::<_, ()>(&mut redis_conn)
            .await?;
    }

    Ok(())
}

pub async fn search_messages(
    query: String,
    session_id: String,
    openai_client: Client,
    mut redis_conn: redis::aio::ConnectionManager,
) -> Result<Vec<RedisearchResult>, Box<dyn std::error::Error>> {
    let request = CreateEmbeddingRequestArgs::default()
        .model("text-embedding-ada-002")
        .input(vec![query])
        .build()?;

    let response = openai_client.embeddings().create(request).await?;
    let vector = encode(response.data[0].embedding.clone());
    let query = format!("@session:{}=>[KNN 10 @vector $V AS dist]", session_id);

    let values: Vec<Value> = redis::cmd("TFT.SEARCH")
        .arg("motorhead")
        .arg(query)
        .arg("PARAMS")
        .arg("2")
        .arg("V")
        .arg(vector)
        .arg("RETURN")
        .arg("3")
        .arg("role")
        .arg("content")
        .arg("dist")
        .arg("SORTBY")
        .arg("dist")
        .arg("DIALECT")
        .arg("2")
        .query_async(&mut redis_conn)
        .await?;

    let array_value = redis::Value::Bulk(values);
    let results = parse_redisearch_response(&array_value);

    Ok(results)
}
