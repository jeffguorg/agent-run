#!/usr/bin/env python3

import argparse
import json
import logging
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

import requests
from bs4 import BeautifulSoup
from bs4.element import Tag


SOURCE_URL = "https://api-docs.deepseek.com/zh-cn/quick_start/pricing"
MODEL_ID_RE = re.compile(r"deepseek-[a-z0-9][a-z0-9.-]*", re.IGNORECASE)
PRICE_RE = re.compile(r"([0-9]+(?:\.[0-9]+)?)元")
SIZE_RE = re.compile(r"([0-9]+(?:\.[0-9]+)?)\s*([KkMm])")
LOG = logging.getLogger("fetch-deepseek-pricing")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Fetch DeepSeek pricing and emit normalized JSON."
    )
    parser.add_argument("--url", default=SOURCE_URL, help="Pricing page URL")
    parser.add_argument(
        "-o",
        "--output",
        type=Path,
        help="Write JSON to this path instead of stdout",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=20.0,
        help="HTTP timeout in seconds",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Enable verbose logging",
    )
    return parser.parse_args()


def fetch_html(url: str, timeout: float) -> str:
    LOG.info("fetching url=%s timeout=%s", url, timeout)
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"):
        value = os.environ.get(key) or os.environ.get(key.lower())
        if value:
            LOG.info("env %s=%s", key, value)

    response = requests.get(
        url,
        timeout=timeout,
        headers={
            "User-Agent": "agent-run deepseek pricing fetcher",
        },
    )
    response.raise_for_status()
    response.encoding = response.apparent_encoding or response.encoding
    LOG.info(
        "fetched status=%s bytes=%s final_url=%s encoding=%s",
        response.status_code,
        len(response.text),
        response.url,
        response.encoding,
    )
    return response.text


def parse_html(html: str) -> BeautifulSoup:
    return BeautifulSoup(html, "html.parser")


def html_lines(soup: BeautifulSoup) -> list[str]:
    main = soup.find("main") or soup.find("article") or soup.body or soup
    text = main.get_text("\n", strip=True)
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    LOG.info("extracted %s text lines from html", len(lines))
    return lines


def split_labeled_values(line: str, label: str) -> list[str]:
    body = line[len(label) :].strip()
    return [value for value in re.split(r"\s{2,}", body) if value]


def parse_size_to_tokens(value: str) -> int | None:
    match = SIZE_RE.search(value)
    if not match:
        return None
    number = float(match.group(1))
    suffix = match.group(2).upper()
    multiplier = 1000 if suffix == "K" else 1_000_000
    return int(number * multiplier)


def extract_single_price(line: str) -> float | None:
    match = PRICE_RE.search(line)
    return float(match.group(1)) if match else None


def extract_double_prices(line: str) -> list[float]:
    return [float(match.group(1)) for match in PRICE_RE.finditer(line)]


def normalize_cell_text(cell: Tag) -> str:
    return cell.get_text(" ", strip=True).replace("\xa0", " ").strip()


def expand_table_rows(table: Tag) -> list[list[str]]:
    expanded: list[list[str]] = []
    rowspan_state: dict[int, tuple[str, int]] = {}

    for row in table.find_all("tr"):
        values: list[str] = []
        col = 0
        cells = row.find_all(["th", "td"], recursive=False)

        def fill_rowspans() -> None:
            nonlocal col
            while col in rowspan_state:
                text, remaining = rowspan_state[col]
                values.append(text)
                if remaining <= 1:
                    del rowspan_state[col]
                else:
                    rowspan_state[col] = (text, remaining - 1)
                col += 1

        fill_rowspans()
        for cell in cells:
            fill_rowspans()
            text = normalize_cell_text(cell)
            colspan = int(cell.get("colspan", 1) or 1)
            rowspan = int(cell.get("rowspan", 1) or 1)
            for offset in range(colspan):
                values.append(text)
                if rowspan > 1:
                    rowspan_state[col + offset] = (text, rowspan - 1)
            col += colspan
            fill_rowspans()

        fill_rowspans()
        expanded.append(values)

    return expanded


def parse_table_models(soup: BeautifulSoup, source_url: str) -> dict | None:
    table = soup.find("table")
    if table is None:
        LOG.info("no pricing table found in html")
        return None

    rows = expand_table_rows(table)
    LOG.info("found pricing table with %s rows", len(rows))
    if not rows:
        return None

    header_values = rows[0]
    LOG.info("table header cells: %s", header_values)
    if len(header_values) < 4:
        return None

    model_ids = [
        match.group(0)
        for value in header_values[2:]
        if (match := MODEL_ID_RE.search(value)) is not None
    ]
    if not model_ids:
        return None
    LOG.info("parsed model ids from table: %s", ", ".join(model_ids))

    records: dict[str, dict] = {
        model_id: {
            "id": model_id,
            "input_cost_per_million": None,
            "output_cost_per_million": None,
            "cached_input_cost_per_million": None,
            "cached_output_cost_per_million": None,
        }
        for model_id in model_ids
    }

    for values in rows[1:]:
        if len(values) < 3:
            continue

        if values[0] in {"功能", "价格"}:
            label = values[1]
            model_values = values[2 : 2 + len(model_ids)]
        else:
            label = values[0]
            model_values = values[1 : 1 + len(model_ids)]
        LOG.debug("table row label=%s values=%s", label, model_values)
        if len(model_values) != len(model_ids):
            continue

        if label == "模型版本":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["name"] = value
        elif label == "思考模式":
            reasoning = any("思考" in value or "支持" in value for value in model_values)
            for record in records.values():
                record["reasoning"] = reasoning
        elif label == "上下文长度":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["context_window"] = parse_size_to_tokens(value)
        elif label == "输出长度":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["max_output_tokens"] = parse_size_to_tokens(value)
        elif label == "百万tokens输入（缓存命中）":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["cached_input_cost_per_million"] = extract_single_price(value)
        elif label == "百万tokens输入（缓存未命中）":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["input_cost_per_million"] = extract_single_price(value)
        elif label == "百万tokens输出":
            for model_id, value in zip(model_ids, model_values, strict=True):
                records[model_id]["output_cost_per_million"] = extract_single_price(value)

    for record in records.values():
        record.setdefault("name", record["id"])
        record.setdefault("context_window", None)
        record.setdefault("max_output_tokens", None)
        record.setdefault("reasoning", True)
        record.setdefault("vision", False)
        record.setdefault("supports_attachments", False)
        record["source"] = {
            "kind": "deepseek-pricing-page",
            "url": source_url,
        }

    return {
        "source_url": source_url,
        "fetched_at": datetime.now(timezone.utc).isoformat(),
        "currency": "CNY",
        "models": list(records.values()),
    }


def parse_models(lines: list[str], source_url: str) -> dict:
    model_line = next(
        (
            line
            for line in lines
            if "deepseek-" in line.lower() and MODEL_ID_RE.search(line) is not None
        ),
        None,
    )
    if not model_line:
        raise RuntimeError("failed to find model list line")
    LOG.info("model line: %s", model_line)

    model_ids = MODEL_ID_RE.findall(model_line)
    if not model_ids:
        raise RuntimeError("failed to parse model ids from pricing page")
    LOG.info("parsed model ids: %s", ", ".join(model_ids))

    records: dict[str, dict] = {
        model_id: {
            "id": model_id,
            "input_cost_per_million": None,
            "output_cost_per_million": None,
            "cached_input_cost_per_million": None,
            "cached_output_cost_per_million": None,
        }
        for model_id in model_ids
    }

    for line in lines:
        if line.startswith("模型版本 "):
            values = split_labeled_values(line, "模型版本")
            LOG.debug("模型版本 values=%s", values)
            if len(values) == len(model_ids):
                for model_id, value in zip(model_ids, values, strict=True):
                    records[model_id]["name"] = value
        elif line.startswith("上下文长度 "):
            value = parse_size_to_tokens(line)
            LOG.debug("上下文长度 value=%s parsed=%s", line, value)
            for record in records.values():
                record["context_window"] = value
        elif line.startswith("输出长度 "):
            value = parse_size_to_tokens(line)
            LOG.debug("输出长度 value=%s parsed=%s", line, value)
            for record in records.values():
                record["max_output_tokens"] = value
        elif line.startswith("思考模式 "):
            reasoning = "支持" in line
            LOG.debug("思考模式 value=%s parsed=%s", line, reasoning)
            for record in records.values():
                record["reasoning"] = reasoning
        elif line.startswith("百万tokens输入（缓存命中）"):
            values = extract_double_prices(line)
            LOG.debug("缓存命中 prices=%s", values)
            if len(values) == len(model_ids):
                for model_id, value in zip(model_ids, values, strict=True):
                    records[model_id]["cached_input_cost_per_million"] = value
        elif line.startswith("百万tokens输入（缓存未命中）"):
            values = extract_double_prices(line)
            LOG.debug("缓存未命中 prices=%s", values)
            if len(values) == len(model_ids):
                for model_id, value in zip(model_ids, values, strict=True):
                    records[model_id]["input_cost_per_million"] = value
        elif line.startswith("百万tokens输出 "):
            values = extract_double_prices(line)
            LOG.debug("输出 prices=%s", values)
            if len(values) == len(model_ids):
                for model_id, value in zip(model_ids, values, strict=True):
                    records[model_id]["output_cost_per_million"] = value

    for record in records.values():
        record.setdefault("name", record["id"])
        record.setdefault("context_window", None)
        record.setdefault("max_output_tokens", None)
        record.setdefault("reasoning", True)
        record.setdefault("vision", False)
        record.setdefault("supports_attachments", False)
        record["source"] = {
            "kind": "deepseek-pricing-page",
            "url": source_url,
        }

    return {
        "source_url": source_url,
        "fetched_at": datetime.now(timezone.utc).isoformat(),
        "currency": "CNY",
        "models": list(records.values()),
    }


def main() -> int:
    args = parse_args()
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(levelname)s %(message)s",
    )
    LOG.info(
        "args url=%s output=%s timeout=%s verbose=%s",
        args.url,
        args.output,
        args.timeout,
        args.verbose,
    )
    html = fetch_html(args.url, args.timeout)
    soup = parse_html(html)
    payload = parse_table_models(soup, args.url)
    if payload is None:
        LOG.info("table parse failed, falling back to line-oriented parse")
        payload = parse_models(html_lines(soup), args.url)
    rendered = json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
        LOG.info("wrote output to %s", args.output)
    else:
        sys.stdout.write(rendered)
        LOG.info("wrote output to stdout")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
