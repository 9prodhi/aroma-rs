class BuildCsvRow {
    String csvRow(String[] cells) {
        StringBuilder sb = new StringBuilder();
        int index = 0;
        for (String cell : cells) {
            if (index > 0) {
                sb.append(',');
            }
            sb.append('"').append(cell.replace("\"", "\"\"")).append('"');
            index++;
        }
        sb.append("\n");
        String result = sb.toString();
        return result;
    }
}
