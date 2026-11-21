import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;

class ReadLinesCount {
    int countLines(String path) throws IOException {
        BufferedReader reader = new BufferedReader(new FileReader(path));
        int count = 0;
        String line = reader.readLine();
        while (line != null) {
            count++;
            line = reader.readLine();
        }
        reader.close();
        return count;
    }
}
