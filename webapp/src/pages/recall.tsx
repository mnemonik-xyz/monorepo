import React, { useState } from "react";

const RecallPage: React.FC = () => {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState("");
  const [loading, setLoading] = useState(false);

  const handleRecall = async () => {
    if (!query.trim()) return;

    setLoading(true);
    setResult("");
    
    try {
      const response = await fetch