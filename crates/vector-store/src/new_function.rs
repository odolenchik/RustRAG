    /// Insert documents into the vector store.
    pub fn insert_documents(&self, documents: &[Document]) -> Result<(), RagCoreError> {
        if documents.is_empty() {
            return Ok(());
        }

        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            std::fs::write(&index_path, "")?;
            Self::restrict_file_permissions(&index_path);
        }

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(index_path)?;

        // Ensure that we start on a new line if the file is not empty and does not end with a newline.
        if file.metadata()?.len() > 0 {
            let mut buffer = [0; 1];
            file.seek(std::io::SeekFrom::End(-1))?;
            file.read_exact(&mut buffer)?;
            if buffer[0] != b'\n' {
                file.write_all(b"\n")?;
            }
            // Seek back to the end for appending
            file.seek(std::io::SeekFrom::End(0))?;
        }

        let mut writer = std::io::BufWriter::new(file);
        for doc in documents {
            let value = serde_json::json!({
                "id": doc.id,
                "file_path": doc.chunk.file_path.to_string_lossy(),
                "line_start": doc.chunk.line_start,
                "line_end": doc.chunk.line_end,
                "module_name": doc.chunk.module_name,
                "symbol_kind": &doc.chunk.symbol_kind,
                "text": doc.chunk.text,
                "embedding": &doc.embedding,
            });
            let line = serde_json::to_string(&value)?;
            writeln!(writer, "{}", line)?;
        }
        writer.flush()?;

        Ok(())
    }
