import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

class ReadLinesCollect {
    List<String> readAll(String path) throws IOException {
        BufferedReader reader = new BufferedReader(new FileReader(path));
        List<String> lines = new ArrayList<>();
        String line = reader.readLine();
        while (line != null) {
            lines.add(line.trim());
            line = reader.readLine();
        }
        reader.close();
        return lines;
    }
}
