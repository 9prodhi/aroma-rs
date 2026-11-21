import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.IOException;
import java.util.List;

class WriteLines {
    int writeLines(String path, List<String> lines) throws IOException {
        BufferedWriter writer = new BufferedWriter(new FileWriter(path));
        int written = 0;
        for (String line : lines) {
            writer.write(line);
            writer.newLine();
            written++;
        }
        writer.flush();
        writer.close();
        return written;
    }
}
