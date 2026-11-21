import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.IOException;
import java.util.List;

class WriteCsv {
    int writeCsv(String path, List<String[]> rows) throws IOException {
        BufferedWriter writer = new BufferedWriter(new FileWriter(path));
        int written = 0;
        for (String[] row : rows) {
            writer.write(String.join(",", row));
            writer.newLine();
            written++;
        }
        writer.flush();
        writer.close();
        return written;
    }
}
